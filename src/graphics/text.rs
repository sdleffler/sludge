use crate::{assets::Cached, graphics::*};

use {
    im::HashMap,
    image::{Rgba, RgbaImage},
    std::io,
};

// AsciiSubset refers to the subset of ascii characters which give alphanumeric characters plus symbols
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    vertical_offset: i32,
    width: f32,
    uvs: Box2<f32>,
}

/// `FontTexture` is a texture generated using the *_character_list functions.
/// It contains a texture representing all of the rasterized characters
/// retrieved from the *_character_list function. `font_map` represents a
/// a mapping between a character and its respective character texture
/// located within `font_texture`.
#[derive(Debug, Clone)]
pub struct FontAtlas {
    font_map: HashMap<char, CharInfo>,
    font_texture: Cached<Texture>,
}

impl FontAtlas {
    pub fn new<R: Read>(
        ctx: &mut Graphics,
        font: R,
        font_size: f32,
        char_list_type: CharacterListType,
    ) -> Result<FontAtlas> {
        use rusttype as rt;

        let inval_bb = rt::Rect {
            min: rt::Point { x: 0, y: 0 },
            max: rt::Point {
                x: (font_size / 4.0) as i32,
                y: 0,
            },
        };
        const MARGIN: u32 = 3;
        let char_list = Self::get_char_list(char_list_type)?;
        let chars_per_row = ((char_list.len() as f32).sqrt() as u32) + 1;
        let bytes_font = font.bytes().collect::<Result<Vec<u8>, io::Error>>()?;
        let rusttype_font = rt::Font::try_from_bytes(&bytes_font[..]).ok_or(anyhow!(
            "Unable to create a rusttype::Font using bytes_font"
        ))?;
        let mut glyphs_and_chars = char_list
            .iter()
            .map(|c| {
                (
                    rusttype_font
                        .glyph(*c)
                        .scaled(rt::Scale {
                            x: font_size,
                            y: font_size,
                        })
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

        for row in char_rows {
            let first_glyph = row.first().unwrap().0;
            let height = first_glyph
                .pixel_bounding_box()
                .unwrap_or(inval_bb)
                .height() as u32;

            for (glyph, c) in row {
                let bb = glyph.pixel_bounding_box().unwrap_or(inval_bb);

                char_map.insert(
                    c,
                    CharInfo {
                        vertical_offset: bb.min.y,
                        uvs: Box2::new(
                            texture_cursor.x as f32 / texture_width as f32,
                            texture_cursor.y as f32 / texture_height as f32,
                            bb.width() as f32 / texture_width as f32,
                            bb.height() as f32 / texture_height as f32,
                        ),
                        width: glyph.unpositioned().h_metrics().advance_width,
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
            font_map: char_map,
            font_texture: Cached::new(Texture::from_rgba8(
                ctx,
                texture_width as u16,
                texture_height as u16,
                &texture,
            )),
        })
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

#[derive(Debug)]
pub struct Text {
    batch: SpriteBatch,
    char_texture_map: HashMap<char, CharInfo>,
}

impl Text {
    pub fn new(ctx: &mut Graphics, input_text: &str, font_atlas: &FontAtlas, color: Color) -> Self {
        let mut text = Text {
            batch: SpriteBatch::with_capacity(
                ctx,
                font_atlas.font_texture.clone(),
                input_text.len(),
            ),
            char_texture_map: font_atlas.font_map.clone(),
        };
        text.set_text(input_text, color);
        text
    }

    pub fn set_text(&mut self, new_text: &str, color: Color) {
        self.batch.clear();
        let mut width: f32 = 0.;
        for c in new_text.chars() {
            let c_info = self
                .char_texture_map
                .get(&c)
                .unwrap_or(self.char_texture_map.get(&'a').unwrap());
            let i_param = InstanceParam::new()
                .src(c_info.uvs)
                .color(color)
                .translate2(Vector2::new(width, c_info.vertical_offset as f32));
            self.batch.insert(i_param);
            width += c_info.width;
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
