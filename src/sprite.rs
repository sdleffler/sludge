use {
    anyhow::*,
    aseprite::SpritesheetData,
    hashbrown::HashMap,
    serde::{Deserialize, Serialize},
    std::{io::Read, ops},
};

use crate::{
    assets::{Asset, Cache, Key, Loaded},
    ecs::*,
    filesystem::Filesystem,
    math::*,
    Resources,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct TagId(u32);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct FrameId(u32);

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub frame: Box2<u32>,
    pub frame_source: Box2<u32>,
    pub source_size: Vector2<u32>,
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

impl ops::Index<FrameId> for SpriteSheet {
    type Output = Frame;

    fn index(&self, FrameId(id): FrameId) -> &Self::Output {
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
            .map(|frame| {
                let fr = frame.frame;
                let sb = frame.sprite_source_size;
                let ss = frame.source_size;

                Frame {
                    frame: Box2::new(fr.x, fr.y, fr.w, fr.h),
                    frame_source: Box2::new(sb.x, sb.y, sb.w, sb.h),
                    source_size: Vector2::new(ss.w, ss.h),
                    duration: frame.duration,
                }
            })
            .collect();

        let dims = spritesheet_data.meta.size;

        Ok(Self {
            image: spritesheet_data
                .meta
                .image
                .ok_or_else(|| anyhow!("no image path"))?,
            tag_ids,
            tags,
            frames,
            size: Vector2::new(dims.w, dims.h),
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpriteFrame(pub FrameId);

/// Component holding the state of a running animation at a given tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        match key {
            Key::Path(path) => {
                let mut fh = resources.fetch_mut::<Filesystem>().open(&path)?;
                let mut buf = String::new();
                fh.read_to_string(&mut buf)?;
                Ok(SpriteSheet::from_json(&buf)?.into())
            } //_ => bail!("can only load from logical"),
        }
    }
}
