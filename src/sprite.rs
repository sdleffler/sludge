use {
    anyhow::*,
    aseprite::SpritesheetData,
    hashbrown::HashMap,
    rlua::prelude::*,
    serde::{Deserialize, Serialize},
    std::{io::Read, ops},
};

use crate::{
    api::{LuaComponent, LuaComponentInterface},
    assets::{Asset, Cache, Cached, DefaultCache, Key, Loaded},
    ecs::*,
    filesystem::Filesystem,
    math::*,
    Resources, SludgeLuaContextExt, SludgeResultExt,
};

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct TagId(u32);

impl<'lua> ToLua<'lua> for TagId {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        self.0.to_lua(lua)
    }
}

impl<'lua> FromLua<'lua> for TagId {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        u32::from_lua(lua_value, lua).map(|i| TagId(i))
    }
}

#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct FrameId(u32);

impl<'lua> ToLua<'lua> for FrameId {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        self.0.to_lua(lua)
    }
}

impl<'lua> FromLua<'lua> for FrameId {
    fn from_lua(lua_value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        u32::from_lua(lua_value, lua).map(|i| FrameId(i))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Direction {
    Forward,
    Reverse,
    Pingpong,
}

impl From<aseprite::Direction> for Direction {
    fn from(ad: aseprite::Direction) -> Self {
        match ad {
            aseprite::Direction::Forward => Self::Forward,
            aseprite::Direction::Reverse => Self::Reverse,
            aseprite::Direction::Pingpong => Self::Pingpong,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub from: u32,
    pub to: u32,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy)]
pub enum NextFrame {
    /// Returned if this is just the next frame ID.
    Stepped(FrameId),
    Wrapped(FrameId),
}

impl Tag {
    pub fn first_frame(&self) -> FrameId {
        match self.direction {
            Direction::Forward | Direction::Pingpong => FrameId(self.from),
            Direction::Reverse => FrameId(self.to),
        }
    }

    pub fn last_frame(&self) -> FrameId {
        match self.direction {
            Direction::Forward | Direction::Pingpong => FrameId(self.to),
            Direction::Reverse => FrameId(self.from),
        }
    }

    /// Returns `Err` if this next frame would loop the animation, `Ok` otherwise.
    pub fn next_frame(&self, FrameId(current): FrameId) -> Result<FrameId, FrameId> {
        match self.direction {
            Direction::Forward if current == self.to => Err(FrameId(self.from)),
            Direction::Reverse if current == self.from => Err(FrameId(self.to)),
            Direction::Pingpong if current == self.to => {
                Err(FrameId(na::max(self.to - 1, self.from)))
            }
            Direction::Pingpong if current == self.from => {
                Err(FrameId(na::min(self.from + 1, self.to)))
            }
            Direction::Forward => Ok(FrameId(current + 1)),
            Direction::Reverse => Ok(FrameId(current - 1)),
            Direction::Pingpong => todo!("pingpong is broken!"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Frame {
    pub frame: Box2<u32>,
    pub frame_source: Box2<u32>,
    pub source_size: Vector2<u32>,
    pub offset: Vector2<f32>,
    pub uvs: Box2<f32>,
    pub duration: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpriteSheet {
    pub image: String,
    pub tag_ids: HashMap<String, TagId>,
    pub tags: Vec<Tag>,
    pub frames: Vec<Frame>,
    pub size: Vector2<u32>,
}

impl ops::Index<TagId> for SpriteSheet {
    type Output = Tag;

    fn index(&self, TagId(id): TagId) -> &Self::Output {
        &self.tags[id as usize]
    }
}

impl ops::Index<SpriteTag> for SpriteSheet {
    type Output = Tag;

    fn index(&self, sprite_tag: SpriteTag) -> &Self::Output {
        &self[sprite_tag.tag_id]
    }
}

impl ops::Index<FrameId> for SpriteSheet {
    type Output = Frame;

    fn index(&self, FrameId(id): FrameId) -> &Self::Output {
        &self.frames[id as usize]
    }
}

impl ops::Index<SpriteFrame> for SpriteSheet {
    type Output = Frame;

    fn index(&self, SpriteFrame(FrameId(id)): SpriteFrame) -> &Self::Output {
        &self.frames[id as usize]
    }
}

impl SpriteSheet {
    pub fn from_reader<R: Read>(reader: &mut R) -> Result<Self> {
        let mut buf = String::new();
        reader.read_to_string(&mut buf)?;
        Self::from_json(&buf)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let spritesheet_data = serde_json::from_str::<SpritesheetData>(s)?;
        let dims = spritesheet_data.meta.size;
        let size = Vector2::new(dims.w, dims.h);

        let tags = spritesheet_data
            .meta
            .frame_tags
            .into_iter()
            .flatten()
            .map(|frame_tag| Tag {
                name: frame_tag.name,
                from: frame_tag.from,
                to: frame_tag.to,
                direction: Direction::from(frame_tag.direction),
            })
            .collect::<Vec<_>>();

        let tag_ids = tags
            .iter()
            .enumerate()
            .map(|(i, tag)| (tag.name.clone(), TagId(i as u32)))
            .collect::<HashMap<_, _>>();

        let frames = spritesheet_data
            .frames
            .into_iter()
            .map(|ase_frame| {
                let fr = ase_frame.frame;
                let sb = ase_frame.sprite_source_size;
                let ss = ase_frame.source_size;

                let duration = ase_frame.duration;
                let frame = Box2::new(fr.x, fr.y, fr.w, fr.h);
                let frame_source = Box2::new(sb.x, sb.y, sb.w, sb.h);
                let source_size = Vector2::new(ss.w, ss.h);
                let offset = (Vector2::new(sb.x as f32, sb.y as f32)
                    - Vector2::new(ss.w as f32, ss.h as f32) / 2.)
                    .map(f32::floor);
                let uvs = Box2::new(
                    fr.x as f32 / size.x as f32,
                    fr.y as f32 / size.y as f32,
                    fr.w as f32 / size.x as f32,
                    fr.h as f32 / size.y as f32,
                );

                Frame {
                    frame,
                    frame_source,
                    source_size,
                    offset,
                    uvs,
                    duration,
                }
            })
            .collect();

        Ok(Self {
            image: spritesheet_data
                .meta
                .image
                .ok_or_else(|| anyhow!("no image path"))?,
            tag_ids,
            tags,
            frames,
            size,
        })
    }

    pub fn update_animation(&self, dt: f32, tag: &mut SpriteTag, frame: &mut SpriteFrame) {
        if let Some((new_tag, maybe_new_frame)) = self.update_animation_inner(dt, tag, frame) {
            *tag = new_tag;

            if let Some(new_frame) = maybe_new_frame {
                *frame = new_frame;
            }
        }
    }

    fn update_animation_inner(
        &self,
        dt: f32,
        tag: &SpriteTag,
        SpriteFrame(frame): &SpriteFrame,
    ) -> Option<(SpriteTag, Option<SpriteFrame>)> {
        if !tag.is_paused {
            let mut new_tag = SpriteTag {
                remaining: tag.remaining - dt * 1_000.,
                ..*tag
            };

            if new_tag.remaining < 0. {
                match self[new_tag.tag_id].next_frame(*frame) {
                    Err(_) if !tag.should_loop => Some((
                        SpriteTag {
                            is_paused: true,
                            ..new_tag
                        },
                        Some(SpriteFrame(self[new_tag.tag_id].last_frame())),
                    )),
                    Ok(new_frame) | Err(new_frame) => {
                        new_tag.remaining += self[new_frame].duration as f32;
                        Some((new_tag, Some(SpriteFrame(new_frame))))
                    }
                }
            } else {
                Some((new_tag, None))
            }
        } else {
            None
        }
    }

    pub fn get_tag<K: AsRef<str>>(&self, s: K) -> Option<TagId> {
        self.tag_ids.get(s.as_ref()).copied()
    }

    pub fn at_tag(&self, tag_id: TagId, should_loop: bool) -> (SpriteFrame, SpriteTag) {
        let tag = &self[tag_id];
        let ff = tag.first_frame();
        (
            SpriteFrame(ff),
            SpriteTag {
                tag_id,
                remaining: self[ff].duration as f32,
                is_paused: false,
                should_loop,
            },
        )
    }
}

/// Component holding the string name of a spritesheet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpriteName(pub String);

/// Component holding the current frame ID of a sprite.
#[derive(Debug, Copy, Clone, Default, Serialize, Deserialize)]
pub struct SpriteFrame(pub FrameId);

/// Component holding the state of a running animation at a given tag.
#[derive(Debug, Copy, Clone, Default, Serialize, Deserialize)]
pub struct SpriteTag {
    /// The index of the currently running animation/tag.
    pub tag_id: TagId,
    /// Remaining time for this frame, in milliseconds.
    pub remaining: f32,
    /// Whether this animation is running or paused.
    pub is_paused: bool,
    /// Whether this animation should loop, or pause on the last frame.
    pub should_loop: bool,
}

impl<'a> SmartComponent<ScContext<'a>> for SpriteName {}
impl<'a> SmartComponent<ScContext<'a>> for SpriteFrame {}
impl<'a> SmartComponent<ScContext<'a>> for SpriteTag {}
impl<'a> SmartComponent<ScContext<'a>> for SpriteSheet {}

impl Asset for SpriteSheet {
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        _cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<Self>> {
        let path = key.to_path()?;
        let mut fh = resources.fetch_mut::<Filesystem>().open(&path)?;
        let mut buf = String::new();
        fh.read_to_string(&mut buf)?;
        Ok(SpriteSheet::from_json(&buf)?.into())
    }
}

#[derive(Debug, Clone)]
pub struct SpriteAnimation {
    pub frame: SpriteFrame,
    pub tag: SpriteTag,
    pub sheet: Cached<SpriteSheet>,
}

impl SpriteAnimation {
    pub fn from_sheet(sheet: Cached<SpriteSheet>) -> Self {
        Self {
            frame: SpriteFrame::default(),
            tag: SpriteTag::default(),
            sheet,
        }
    }

    pub fn update(&mut self, dt: f32) -> Frame {
        let sheet = self.sheet.load_cached();
        sheet.update_animation(dt, &mut self.tag, &mut self.frame);
        sheet[self.frame.0]
    }
}

impl<'a> SmartComponent<ScContext<'a>> for SpriteAnimation {}

#[derive(Debug, Clone, Copy)]
pub struct SpriteAnimationAccessor(Entity);

impl LuaUserData for SpriteAnimationAccessor {
    fn add_methods<'lua, T: LuaUserDataMethods<'lua, Self>>(_methods: &mut T) {}
}

impl LuaComponentInterface for SpriteAnimation {
    fn accessor<'lua>(lua: LuaContext<'lua>, entity: Entity) -> LuaResult<LuaValue<'lua>> {
        SpriteAnimationAccessor(entity).to_lua(lua)
    }

    fn bundler<'lua>(
        lua: LuaContext<'lua>,
        args: LuaValue<'lua>,
        builder: &mut EntityBuilder,
    ) -> LuaResult<()> {
        let resources = lua.resources();

        let table = LuaTable::from_lua(args, lua)?;
        let path = table
            .get::<_, LuaString>("path")
            .log_error_err(module_path!())?;
        let mut sprite_sheet = resources
            .fetch_mut::<DefaultCache>()
            .get::<SpriteSheet>(&Key::from_path(path.to_str()?))
            .to_lua_err()?;

        let should_loop = table.get::<_, Option<bool>>("should_loop")?.unwrap_or(true);

        let tag_id = match table
            .get::<_, Option<LuaString>>("tag")
            .log_warn_err(module_path!())?
        {
            Some(tag_name) => sprite_sheet.load_cached().get_tag(tag_name.to_str()?),
            None => None,
        }
        .unwrap_or_default();
        let (frame, tag) = sprite_sheet.load_cached().at_tag(tag_id, should_loop);

        builder.add(SpriteAnimation {
            frame,
            tag,
            sheet: sprite_sheet,
        });
        Ok(())
    }
}

inventory::submit! {
    LuaComponent::new::<SpriteAnimation>("SpriteAnimation")
}
