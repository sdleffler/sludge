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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontAtlasKey<'a> {
    pub path: Cow<'a, Path>,
    pub size: u32,
    pub char_list_type: CharacterListType,
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
    font_texture: Cached<Texture>,
    font_map: HashMap<char, CharInfo>,
}

impl FontAtlas {
    pub(crate) fn from_rusttype_font(
        ctx: &mut Graphics,
        rusttype_font: &rusttype::Font,
        em_pixels: f32,
        char_list_type: CharacterListType,
    ) -> Result<FontAtlas> {
        use rusttype as rt;

        let font_scale = rt::Scale {
            x: em_pixels,
            y: em_pixels,
        };
        let inval_bb = rt::Rect {
            min: rt::Point { x: 0, y: 0 },
            max: rt::Point {
                x: (em_pixels / 4.0) as i32,
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
                        vertical_offset: (v_metrics.ascent + bb.min.y as f32) / em_pixels,
                        uvs: Box2::new(
                            texture_cursor.x as f32 / texture_width as f32,
                            texture_cursor.y as f32 / texture_height as f32,
                            bb.width() as f32 / texture_width as f32,
                            bb.height() as f32 / texture_height as f32,
                        ),
                        advance_width: h_metrics.advance_width / em_pixels,
                        horizontal_offset: h_metrics.left_side_bearing / em_pixels,
                        scale: Vector2::repeat(1. / em_pixels),
                    },
                );

                glyph.draw(|x, y, v| {
                    let x: u32 = texture_cursor.x as u32 + x;
                    let y: u32 = texture_cursor.y as u32 + y;
                    let c = (v * 255.0) as u8;
                    let color = Rgba([255, 255, 255, c]);
                    texture.put_pixel(x, y, color);
                });

                texture_cursor.x += (bb.width() as u32) + MARGIN;
            }
            texture_cursor.y += height + MARGIN;
            texture_cursor.x = 0;
        }

        Ok(FontAtlas {
            font_texture: Cached::new(Texture::from_rgba8(
                ctx,
                texture_width as u16,
                texture_height as u16,
                &texture,
            )),
            font_map: char_map,
        })
    }

    pub fn from_reader<R: Read>(
        ctx: &mut Graphics,
        mut font: R,
        em_pixels: f32,
        char_list_type: CharacterListType,
    ) -> Result<FontAtlas> {
        use rusttype as rt;

        let mut bytes_font = Vec::new();
        font.read_to_end(&mut bytes_font)?;
        let rusttype_font = rt::Font::try_from_bytes(&bytes_font[..]).ok_or(anyhow!(
            "Unable to create a rusttype::Font using bytes_font"
        ))?;

        Self::from_rusttype_font(ctx, &rusttype_font, em_pixels, char_list_type)
    }

    fn get_char_list(char_list_type: CharacterListType) -> Result<Vec<char>> {
        let char_list = match char_list_type {
            CharacterListType::AsciiSubset => [0x20..0x7F].iter(),
            CharacterListType::Ascii => [0x00..0xFF].iter(),
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
    atlas: Cached<FontAtlas>,
}

impl Text {
    pub fn from_cached(ctx: &mut Graphics, font_atlas: Cached<FontAtlas>) -> Self {
        Self::from_cached_with_capacity(ctx, font_atlas, DEFAULT_TEXT_BUFFER_SIZE)
    }

    pub fn from_cached_with_capacity(
        ctx: &mut Graphics,
        mut font_atlas: Cached<FontAtlas>,
        capacity: usize,
    ) -> Self {
        let atlas = font_atlas.load_cached();
        Text {
            batch: SpriteBatch::with_capacity(ctx, atlas.font_texture.clone(), capacity),
            atlas: font_atlas,
        }
    }

    pub fn set_text(&mut self, new_text: &str, color: Color) {
        let atlas = self.atlas.load_cached();
        let font_map = &atlas.font_map;
        self.batch.clear();
        self.batch.set_texture(atlas.font_texture.clone());
        let mut width: f32 = 0.;
        for c in new_text.chars() {
            let c_info = font_map.get(&c).unwrap_or(font_map.get(&'?').unwrap());
            let i_param = InstanceParam::new()
                .src(c_info.uvs)
                .color(color)
                .translate2(Vector2::new(
                    width + c_info.horizontal_offset,
                    c_info.vertical_offset,
                ))
                .scale2(c_info.scale);
            self.batch.insert(i_param);
            width += c_info.advance_width;
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
        let atlas = FontAtlas::from_rusttype_font(
            gfx,
            &font.load_cached().inner,
            key.size as f32,
            key.char_list_type,
        )?;
        Ok(Loaded::with_deps(
            atlas,
            vec![Key::from(key.path.into_owned())],
        ))
    }
}
