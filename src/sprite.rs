use {
    anyhow::*,
    aseprite::SpritesheetData,
    hashbrown::HashMap,
    serde::{Deserialize, Serialize},
    std::ops,
};

use crate::{ecs::*, math::*};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TagId(u32);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

impl Tag {
    pub fn first_frame(&self) -> FrameId {
        match self.direction {
            Direction::Forward | Direction::Pingpong => FrameId(self.from),
            Direction::Reverse => FrameId(self.to),
        }
    }

    pub fn next_frame(&self, FrameId(current): FrameId) -> FrameId {
        match self.direction {
            Direction::Forward if current == self.to => FrameId(self.from),
            Direction::Reverse if current == self.from => FrameId(self.to),
            Direction::Pingpong if current == self.to => FrameId(na::max(self.to - 1, self.from)),
            Direction::Pingpong if current == self.from => FrameId(na::min(self.from + 1, self.to)),
            _ => FrameId(current + 1),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub bounds: Box2<u32>,
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
                let r = frame.frame;

                Frame {
                    bounds: Box2::new(r.x, r.y, r.w, r.h),
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

    pub fn update_animation(
        &self,
        dt: f32,
        tag: &mut SpriteTag,
        SpriteFrame(frame): &mut SpriteFrame,
    ) {
        if !tag.is_paused {
            tag.remaining -= dt * 1_000.;

            if tag.remaining < 0. {
                *frame = self[tag.tag_id].next_frame(*frame);
                tag.remaining += self[*frame].duration as f32;
            }
        }
    }

    pub fn get_tag<K: AsRef<str>>(&self, s: K) -> Option<TagId> {
        self.tag_ids.get(s.as_ref()).copied()
    }

    pub fn at_tag(&self, tag_id: TagId) -> (SpriteFrame, SpriteTag) {
        let tag = &self[tag_id];
        let ff = tag.first_frame();
        (
            SpriteFrame(ff),
            SpriteTag {
                tag_id,
                remaining: self[ff].duration as f32,
                is_paused: false,
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
}

impl<'a> SmartComponent<&'a Flags> for SpriteName {}
impl<'a> SmartComponent<&'a Flags> for SpriteFrame {}
impl<'a> SmartComponent<&'a Flags> for SpriteTag {}
