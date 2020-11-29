use crate::{
    assets::{Asset, Cache, Cached, Key, Loaded},
    filesystem::Filesystem,
    graphics::*,
    Resources,
};

use {
    hashbrown::HashMap,
    image::{Rgba, RgbaImage},
    std::{borrow::Cow, ffi::OsStr, path::Path},
};

#[derive(Debug, Clone)]
pub struct Font {
    inner: rusttype::Font<'static>,
}

// AsciiSubset refers to the subset of ascii characters which give alphanumeric characters plus symbols
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CharacterListType {
    AsciiSubset,
    Ascii,
    ExtendedAscii,
    Cyrillic,
    Thai,
    Vietnamese,
    Chinese,
    Japanese,
}

#[derive(Debug, Clone, Copy)]
struct CharInfo {
    vertical_offset: f32,
    horizontal_offset: f32,
    advance_width: f32,
    uvs: Box2<f32>,
    scale: Vector2<f32>,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ThresholdFunction {
    Above(f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontAtlasKey<'a> {
    pub path: Cow<'a, Path>,
    pub size: u32,
    pub char_list_type: CharacterListType,
    pub threshold: Option<f32>,
}

impl<'a> FontAtlasKey<'a> {
    pub fn new<S: AsRef<OsStr> + ?Sized>(
        path: &'a S,
        size: u32,
        char_list_type: CharacterListType,
    ) -> Self {
        Self {
            path: Cow::Borrowed(Path::new(path)),
            size,
            char_list_type,
            threshold: None,
        }
    }

    pub fn with_threshold<S: AsRef<OsStr> + ?Sized>(
        path: &'a S,
        size: u32,
        char_list_type: CharacterListType,
        threshold: f32,
    ) -> Self {
        Self {
            path: Cow::Borrowed(Path::new(path)),
            size,
            char_list_type,
            threshold: Some(threshold),
        }
    }
}

/// `FontTexture` is a texture generated using the *_character_list functions.
/// It contains a texture representing all of the rasterized characters
/// retrieved from the *_character_list function. `font_map` represents a
/// a mapping between a character and its respective character texture
/// located within `font_texture`.
#[derive(Debug, Clone)]
pub struct FontAtlas {
    pub font_texture: Cached<Texture>,
    font_map: HashMap<char, CharInfo>,
    line_gap: f32,
}

impl FontAtlas {
    pub(crate) fn from_rusttype_font<F: FnMut(f32) -> f32>(
        ctx: &mut Graphics,
        rusttype_font: &rusttype::Font,
        height_px: f32,
        char_list_type: CharacterListType,
        mut threshold: F,
    ) -> Result<FontAtlas> {
        use rusttype as rt;

        let font_scale = rt::Scale::uniform(height_px);
        let inval_bb = rt::Rect {
            min: rt::Point { x: 0, y: 0 },
            max: rt::Point {
                x: (height_px / 4.0) as i32,
                y: 0,
            },
        };
        const MARGIN: u32 = 1;
        let char_list = Self::get_char_list(char_list_type)?;
        let chars_per_row = ((char_list.len() as f32).sqrt() as u32) + 1;
        let mut glyphs_and_chars = char_list
            .iter()
            .map(|c| {
                (
                    rusttype_font
                        .glyph(*c)
                        .scaled(font_scale)
                        .positioned(rt::Point { x: 0.0, y: 0.0 }),
                    *c,
                )
            })
            .collect::<Vec<(rt::PositionedGlyph, char)>>();
        glyphs_and_chars
            .sort_unstable_by_key(|g| g.0.pixel_bounding_box().unwrap_or(inval_bb).height());

        let mut texture_height = glyphs_and_chars
            .last()
            .unwrap()
            .0
            .pixel_bounding_box()
            .unwrap_or(inval_bb)
            .height() as u32;
        let mut current_row = 0;
        let mut widest_row = 0u32;
        let mut row_sum = 0u32;

        // Sort the glyphs by height so that we know how tall each row should be in the atlas
        // Sums all the widths and heights of the bounding boxes so we know how large the atlas will be
        let mut char_rows = Vec::new();
        let mut cur_row = Vec::with_capacity(chars_per_row as usize);

        for (glyph, c) in glyphs_and_chars.iter().rev() {
            let bb = glyph.pixel_bounding_box().unwrap_or(inval_bb);

            if current_row > chars_per_row {
                current_row = 0;
                texture_height += bb.height() as u32;
                if row_sum > widest_row {
                    widest_row = row_sum;
                }
                row_sum = 0;
                char_rows.push(cur_row.clone());
                cur_row.clear();
            }

            cur_row.push((glyph, *c));
            row_sum += bb.width() as u32;
            current_row += 1;
        }
        // Push remaining chars
        char_rows.push(cur_row);

        let texture_width = widest_row + (chars_per_row * MARGIN);
        texture_height += chars_per_row * MARGIN;

        let mut texture = RgbaImage::new(texture_width as u32, texture_height as u32);
        let mut texture_cursor = Point2::<u32>::new(0, 0);
        let mut char_map: HashMap<char, CharInfo> = HashMap::new();
        let v_metrics = rusttype_font.v_metrics(font_scale);

        for row in char_rows {
            let first_glyph = row.first().unwrap().0;
            let height = first_glyph
                .pixel_bounding_box()
                .unwrap_or(inval_bb)
                .height() as u32;

            for (glyph, c) in row {
                let bb = glyph.pixel_bounding_box().unwrap_or(inval_bb);
                let h_metrics = glyph.unpositioned().h_metrics();

                char_map.insert(
                    c,
                    CharInfo {
                        vertical_offset: (v_metrics.ascent + bb.min.y as f32),
                        uvs: Box2::new(
                            texture_cursor.x as f32 / texture_width as f32,
                            texture_cursor.y as f32 / texture_height as f32,
                            bb.width() as f32 / texture_width as f32,
                            bb.height() as f32 / texture_height as f32,
                        ),
                        advance_width: h_metrics.advance_width,
                        horizontal_offset: h_metrics.left_side_bearing,
                        scale: Vector2::repeat(1. / height_px),
                        width: bb.width() as f32,
                        height: bb.height() as f32,
                    },
                );

                glyph.draw(|x, y, v| {
                    let x: u32 = texture_cursor.x as u32 + x;
                    let y: u32 = texture_cursor.y as u32 + y;
                    let c = (threshold(v).clamp(0., 1.) * 255.0) as u8;
                    let color = Rgba([255, 255, 255, c]);
                    texture.put_pixel(x, y, color);
                });

                texture_cursor.x += (bb.width() as u32) + MARGIN;
            }
            texture_cursor.y += height + MARGIN;
            texture_cursor.x = 0;
        }

        let texture_obj =
            Texture::from_rgba8(ctx, texture_width as u16, texture_height as u16, &texture);

        Ok(FontAtlas {
            font_texture: Cached::new(texture_obj),
            font_map: char_map,
            line_gap: v_metrics.ascent - v_metrics.descent + v_metrics.line_gap,
        })
    }

    pub fn from_reader<R: Read>(
        ctx: &mut Graphics,
        mut font: R,
        height_px: f32,
        char_list_type: CharacterListType,
    ) -> Result<FontAtlas> {
        use rusttype as rt;

        let mut bytes_font = Vec::new();
        font.read_to_end(&mut bytes_font)?;
        let rusttype_font = rt::Font::try_from_bytes(&bytes_font[..]).ok_or(anyhow!(
            "Unable to create a rusttype::Font using bytes_font"
        ))?;

        Self::from_rusttype_font(ctx, &rusttype_font, height_px, char_list_type, |v| v)
    }

    fn get_char_list(char_list_type: CharacterListType) -> Result<Vec<char>> {
        let char_list = match char_list_type {
            CharacterListType::AsciiSubset => [0x20..0x7F].iter(),
            CharacterListType::Ascii => [0x00..0x7F].iter(),
            CharacterListType::ExtendedAscii => [0x00..0xFF].iter(),
            CharacterListType::Cyrillic => [
                0x0020u32..0x00FF, // Basic Latin + Latin Supplement
                0x0400u32..0x052F, // Cyrillic + Cyrillic Supplement
                0x2DE0u32..0x2DFF, // Cyrillic Extended-A
                0xA640u32..0xA69F, // Cyrillic Extended-B
            ]
            .iter(),
            CharacterListType::Thai => [
                0x0020u32..0x00FF, // Basic Latin
                0x2010u32..0x205E, // Punctuations
                0x0E00u32..0x0E7F, // Thai
            ]
            .iter(),

            CharacterListType::Vietnamese => [
                0x0020u32..0x00FF, // Basic Latin
                0x0102u32..0x0103,
                0x0110u32..0x0111,
                0x0128u32..0x0129,
                0x0168u32..0x0169,
                0x01A0u32..0x01A1,
                0x01AFu32..0x01B0,
                0x1EA0u32..0x1EF9,
            ]
            .iter(),
            CharacterListType::Chinese => bail!("Chinese fonts not yet supported"),
            CharacterListType::Japanese => bail!("Japanese fonts not yet supported"),
        };
        char_list
            .cloned()
            .flatten()
            .map(|c| {
                std::char::from_u32(c).ok_or(anyhow!("Unable to convert u32 \"{}\" into char", c))
            })
            .collect::<Result<Vec<char>>>()
    }
}

impl Drawable for FontAtlas {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam) {
        self.font_texture.load().draw(ctx, instance);
    }

    fn aabb2(&self) -> Box2<f32> {
        self.font_texture.load().aabb2()
    }
}

const DEFAULT_TEXT_BUFFER_SIZE: usize = 64;

#[derive(Debug)]
pub struct Text {
    batch: SpriteBatch,
}

impl Text {
    pub fn from_cached<T: Clone + Into<Cached<Texture>>>(ctx: &mut Graphics, texture: T) -> Self {
        Self::from_cached_with_capacity(ctx, DEFAULT_TEXT_BUFFER_SIZE, texture)
    }

    pub fn from_cached_with_capacity<T: Clone + Into<Cached<Texture>>>(
        ctx: &mut Graphics,
        capacity: usize,
        texture: T,
    ) -> Self {
        Text {
            batch: SpriteBatch::with_capacity(ctx, texture, capacity),
        }
    }

    fn set_text(&mut self, layout: &TextLayout) {
        self.batch.clear();
        self.batch
            .set_texture(layout.font_atlas.font_texture.clone());
        for layout_c in layout.chars.iter() {
            let c_info = layout
                .font_atlas
                .font_map
                .get(&layout_c.c)
                .unwrap_or(&layout.font_atlas.font_map[&'?']);
            let i_param = InstanceParam::new()
                .src(c_info.uvs)
                .color(layout_c.color)
                .translate2(Vector2::new(layout_c.coords.mins.x, layout_c.coords.mins.y));
            self.batch.insert(i_param);
        }
    }
}

impl Drawable for Text {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam) {
        self.batch.draw(ctx, instance);
    }

    fn aabb2(&self) -> Box2<f32> {
        self.batch.aabb2()
    }
}

// end - ending index of current word within TextLayout.chars (we always
// start at 0 and will use the previous word's end to figure out the size
// of the next word)
// width - width of the given word in pixels (used to determine whether
// or not we should start a new line)
struct Word {
    end: usize,
    width: f32,
}

impl Word {
    fn from_str(
        text: &str,
        font_map: &HashMap<char, CharInfo>,
        mut upper_bound: usize,
    ) -> Vec<Self> {
        let mut buffer = Vec::new();
        for word in text.split(" ") {
            upper_bound += word.len();
            buffer.push(Word {
                end: upper_bound,
                width: word
                    .chars()
                    .map(|c| font_map.get(&c).unwrap_or(&font_map[&'?']).advance_width)
                    .sum(),
            })
        }
        buffer
    }
}

#[derive(Debug)]
pub struct LayoutCharInfo {
    coords: Box2<f32>,
    color: Color,
    c: char,
}

pub struct TextLayout {
    chars: Vec<LayoutCharInfo>,
    words: Vec<Word>,
    font_atlas: FontAtlas,
    cursor: Point2<f32>,
    space_width: f32,
}

impl TextLayout {
    pub fn new(font_atlas: FontAtlas) -> Self {
        let space_width = font_atlas.font_map[&' '].advance_width;
        TextLayout {
            font_atlas: font_atlas,
            chars: Vec::new(),
            words: Vec::new(),
            cursor: Point2::new(0., 0.),
            space_width: space_width,
        }
    }

    pub fn push_str(&mut self, text: &str, colors: impl IntoIterator<Item = Color> + Clone) {
        if let Some(upper_bound) = colors.clone().into_iter().size_hint().1 {
            assert!(
                upper_bound < text.len(),
                "Passed in less colors than the number of chars you tried to push!"
            );
        }
        self.words.append(&mut Word::from_str(
            text,
            &self.font_atlas.font_map,
            self.words.last().unwrap_or(&Word { end: 0, width: 0. }).end,
        ));
        for (c, color) in text.chars().zip(colors.clone()) {
            if c.is_whitespace() {
                self.cursor.x += self.space_width;
                continue;
            }
            let c_info = self
                .font_atlas
                .font_map
                .get(&c)
                .unwrap_or(&self.font_atlas.font_map[&'?']);
            self.chars.push(LayoutCharInfo {
                coords: Box2::new(
                    self.cursor.x + c_info.horizontal_offset,
                    self.cursor.y + c_info.vertical_offset,
                    c_info.width,
                    c_info.height,
                ),
                color,
                c,
            });
            self.cursor.x += c_info.advance_width;
        }
    }

    pub fn push_wrapping_str(
        &mut self,
        text: &str,
        colors: impl IntoIterator<Item = Color> + Clone,
        line_width: f32,
    ) {
        if let Some(upper_bound) = colors.clone().into_iter().size_hint().1 {
            assert!(
                upper_bound < text.len(),
                "Passed in less colors than the number of chars you tried to push!"
            );
        }

        let new_words = Word::from_str(
            text,
            &self.font_atlas.font_map,
            self.words.last().unwrap_or(&Word { end: 0, width: 0. }).end,
        );

        let mut start = match self.words.last() {
            Some(w) => w.end,
            None => 0usize,
        };

        let mut char_iter = text.chars();
        let mut color_iter = colors.into_iter();

        for word in new_words.iter() {
            if word.width + self.cursor.x > line_width as f32 {
                self.cursor.x = 0.;
                self.cursor.y += self.font_atlas.line_gap;
            }

            for _ in 0..(word.end - start) {
                let c = char_iter.next().unwrap();
                let color = color_iter.next().unwrap();
                let c_info = self
                    .font_atlas
                    .font_map
                    .get(&c)
                    .unwrap_or(&self.font_atlas.font_map[&'?']);
                self.chars.push(LayoutCharInfo {
                    coords: Box2::new(
                        self.cursor.x + c_info.horizontal_offset,
                        self.cursor.y + c_info.vertical_offset,
                        c_info.width,
                        c_info.height,
                    ),
                    color,
                    c,
                });
                self.cursor.x += c_info.advance_width;
            }

            start = word.end;
            // Advance the char and color iterators to get rid of the space
            char_iter.next();
            color_iter.next();
            self.cursor.x += self.space_width;
        }
    }

    pub fn apply_layout(&self, gfx: &mut Graphics) -> Text {
        let mut text = Text::from_cached(gfx, self.font_atlas.font_texture.clone());
        text.set_text(&self);
        text
    }
}

impl Asset for Font {
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        _cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<Self>> {
        use rusttype as rt;
        let path = key.to_path()?;
        let mut fs = resources.fetch_mut::<Filesystem>();
        let mut file = fs.open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        let font = rt::Font::try_from_vec(buf).ok_or_else(|| anyhow!("error parsing font"))?;
        Ok(Loaded::new(Font { inner: font }))
    }
}

impl Asset for FontAtlas {
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<Self>> {
        let key = key.to_rust::<FontAtlasKey>()?;
        let mut font = cache.get::<Font>(&Key::from_path(&key.path))?;
        let gfx = &mut *resources.fetch_mut::<Graphics>();
        let atlas = match key.threshold {
            Some(t) => FontAtlas::from_rusttype_font(
                gfx,
                &font.load_cached().inner,
                key.size as f32,
                key.char_list_type,
                |v| if v > t { 1. } else { 0. },
            )?,
            None => FontAtlas::from_rusttype_font(
                gfx,
                &font.load_cached().inner,
                key.size as f32,
                key.char_list_type,
                |v| v,
            )?,
        };
        Ok(Loaded::with_deps(
            atlas,
            vec![Key::from(key.path.into_owned())],
        ))
    }
}
