//! Reads PDF text and images with pdfium, statically linked (see
//! `scripts/fetch-pdfium.sh`). Characters are grouped into visual lines with
//! their boxes and font sizes, and raster images are listed with where they
//! sit; `eddy_core` turns those into a reflowed document. Nothing here knows
//! about columns, headers or which images are worth keeping.

use std::cell::OnceCell;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;

use eddy_core::{PdfImage, PdfLine, PdfReader, PdfText};
use pdfium_render::prelude::*;

/// `PdfReader` backed by pdfium. Cheap to construct; the library is
/// initialised once per process on first use.
#[derive(Clone, Copy, Debug, Default)]
pub struct PdfiumReader;

impl PdfiumReader {
    pub fn new() -> Self {
        Self
    }

    /// Reads the PDF at `path`; `None` when pdfium cannot open it.
    pub fn read_file(&self, path: &Path) -> Option<PdfText> {
        let pdfium = pdfium()?;
        let document = pdfium.load_pdf_from_file(path, None).ok()?;
        Some(read_document(&document))
    }

    /// Image `index` on page `page` of the PDF at `path`, encoded: JPEG data
    /// as stored when a browser can show it unchanged, otherwise PNG.
    pub fn read_image(&self, path: &Path, page: u32, index: u32) -> Option<(Vec<u8>, &'static str)> {
        let pdfium = pdfium()?;
        let document = pdfium.load_pdf_from_file(path, None).ok()?;
        let page = document.pages().get(i32::try_from(page).ok()?).ok()?;
        let page = PageContext::new(page);
        match locate(&document, &page, index)? {
            Picture::Encoded(bytes, mime) => Some((bytes, mime)),
            Picture::Region(bounds) => {
                let scale = render_scale(bounds.width());
                let bitmap = render_page(&page.page, scale)?;
                crop_region(&page.page, &bitmap, scale, bounds).map(|png| (png, "image/png"))
            }
        }
    }

    /// Several images at once, `(page, index)` each, in request order. The
    /// file is opened once, and the pictures on a page that are shown by
    /// rendering its area share one rendering of the page per scale.
    pub fn read_images(&self, path: &Path, requests: &[(u32, u32)]) -> Vec<Option<(Vec<u8>, &'static str)>> {
        let mut out: Vec<Option<(Vec<u8>, &'static str)>> = vec![None; requests.len()];
        let Some(pdfium) = pdfium() else { return out };
        let Ok(document) = pdfium.load_pdf_from_file(path, None) else {
            return out;
        };
        let mut by_page: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (i, (page, _)) in requests.iter().enumerate() {
            by_page.entry(*page).or_default().push(i);
        }
        for (page_number, wanted) in by_page {
            let Ok(page_number) = i32::try_from(page_number) else { continue };
            let Ok(page) = document.pages().get(page_number) else { continue };
            let page = PageContext::new(page);
            // Regions grouped by rendering scale, in first-seen order.
            let mut regions: Vec<(u32, Vec<(usize, Rect)>)> = Vec::new();
            for i in wanted {
                match locate(&document, &page, requests[i].1) {
                    Some(Picture::Encoded(bytes, mime)) => out[i] = Some((bytes, mime)),
                    Some(Picture::Region(bounds)) => {
                        let key = render_scale(bounds.width()).to_bits();
                        match regions.iter_mut().find(|(k, _)| *k == key) {
                            Some((_, group)) => group.push((i, bounds)),
                            None => regions.push((key, vec![(i, bounds)])),
                        }
                    }
                    None => {}
                }
            }
            for (key, group) in regions {
                let scale = f32::from_bits(key);
                let Some(bitmap) = render_page(&page.page, scale) else { continue };
                for (i, bounds) in group {
                    out[i] = crop_region(&page.page, &bitmap, scale, bounds).map(|png| (png, "image/png"));
                }
            }
        }
        out
    }
}

impl PdfReader for PdfiumReader {
    fn read(&self, path: &Path) -> Option<PdfText> {
        self.read_file(path)
    }

    fn image(&self, path: &Path, page: u32, index: u32) -> Option<(Vec<u8>, &'static str)> {
        self.read_image(path, page, index)
    }

    fn images(&self, path: &Path, requests: &[(u32, u32)]) -> Vec<Option<(Vec<u8>, &'static str)>> {
        self.read_images(path, requests)
    }
}

/// Raster rectangles on a page and the groups of them that make one picture.
struct Rasters {
    rects: Vec<(u32, Rect)>,
    groups: Vec<Vec<usize>>,
}

/// A page with what locating its pictures needs, computed once per page.
struct PageContext<'a> {
    page: PdfPage<'a>,
    lines: OnceCell<Vec<PdfLine>>,
    rasters: OnceCell<Rasters>,
}

impl<'a> PageContext<'a> {
    fn new(page: PdfPage<'a>) -> Self {
        Self {
            page,
            lines: OnceCell::new(),
            rasters: OnceCell::new(),
        }
    }

    fn lines(&self) -> &[PdfLine] {
        self.lines.get_or_init(|| page_lines(&self.page))
    }

    fn rasters(&self) -> &Rasters {
        self.rasters.get_or_init(|| {
            let rects = raster_rects(&self.page);
            let groups = tile_groups(&rects);
            Rasters { rects, groups }
        })
    }
}

/// How an image address is shown: bytes as stored or encoded from the
/// object's own pixels, or an area of the page to render.
enum Picture {
    Encoded(Vec<u8>, &'static str),
    Region(Rect),
}

fn locate(document: &PdfDocument<'_>, page: &PageContext<'_>, index: u32) -> Option<Picture> {
    let object = page.page.objects().get(index as usize).ok()?;
    if let Some(form) = object.as_x_object_form_object() {
        let bounds = figure_bounds(form, &page.page, page.lines())?;
        return Some(Picture::Region(bounds));
    }
    let image = object.as_image_object()?;
    if is_upright(&page.page) {
        let Rasters { rects, groups } = page.rasters();
        let group = groups.iter().find(|g| g.iter().any(|&i| rects[i].0 == index));
        if let Some(group) = group.filter(|g| g.len() > 1) {
            let bounds = group.iter().map(|&i| rects[i].1).reduce(Rect::union)?;
            return Some(Picture::Region(bounds));
        }
    }
    if let Some(jpeg) = plain_jpeg(image) {
        return Some(Picture::Encoded(jpeg, "image/jpeg"));
    }
    let bitmap = image.get_processed_bitmap(document).ok()?;
    let (width, height) = (bitmap.width(), bitmap.height());
    encode_png(&bitmap, (0, 0, width, height)).map(|png| Picture::Encoded(png, "image/png"))
}

fn pdfium() -> Option<&'static Pdfium> {
    static PDFIUM: OnceLock<Option<Pdfium>> = OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            Pdfium::bind_to_statically_linked_library()
                .ok()
                .map(Pdfium::new)
        })
        .as_ref()
}

fn read_document(document: &PdfDocument<'_>) -> PdfText {
    let metadata = document.metadata();
    let tag = |t: PdfDocumentMetadataTagType| {
        metadata
            .get(t)
            .map(|v| v.value().trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let pages = document
        .pages()
        .iter()
        .map(|page| read_page(&page))
        .collect();
    PdfText {
        title: tag(PdfDocumentMetadataTagType::Title),
        author: tag(PdfDocumentMetadataTagType::Author),
        pages,
    }
}

fn read_page(page: &PdfPage<'_>) -> eddy_core::PdfPage {
    let width = page.width().value;
    let height = page.height().value;
    let lines = page_lines(page);
    let images = read_images(page, &lines);
    eddy_core::PdfPage {
        width,
        height,
        lines,
        images,
    }
}

fn page_lines(page: &PdfPage<'_>) -> Vec<PdfLine> {
    match page.text() {
        Ok(text) => LineBuilder::default().run(&text),
        Err(_) => Vec::new(),
    }
}

fn is_upright(page: &PdfPage<'_>) -> bool {
    matches!(page.rotation(), Ok(PdfPageRenderRotation::None))
}

/// Raster images drawn directly on the page, and included graphics (form
/// XObjects holding images or vector drawings, as pdflatex embeds figures),
/// which are shown by rendering their area of the page. Rasters that touch
/// or overlap make one picture: PowerPoint exports a large image as strips,
/// and animation steps stack images in one place.
fn read_images(page: &PdfPage<'_>, lines: &[PdfLine]) -> Vec<PdfImage> {
    let upright = is_upright(page);
    let mut images = Vec::new();
    let mut rasters: Vec<PdfImage> = Vec::new();
    for (index, object) in page.objects().iter().enumerate() {
        if let Some(form) = object.as_x_object_form_object() {
            if !upright {
                continue;
            }
            let mut contents = FormContents::default();
            contents.add(form);
            if !contents.is_graphic() {
                continue;
            }
            let Some(bounds) = figure_bounds(form, page, lines) else {
                continue;
            };
            let (width, height) = (bounds.width(), bounds.height());
            let scale = render_scale(width);
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (width.round() as i32, height.round() as i32).hash(&mut hasher);
            contents.hash(&mut hasher);
            images.push(PdfImage {
                index: index as u32,
                x: bounds.left,
                y: bounds.bottom,
                width,
                height,
                pixel_width: (width * scale) as u32,
                pixel_height: (height * scale) as u32,
                fingerprint: hasher.finish(),
            });
            continue;
        }
        let Some(image) = object.as_image_object() else {
            continue;
        };
        let Ok(bounds) = object.bounds() else { continue };
        let (Ok(pixel_width), Ok(pixel_height)) = (image.width(), image.height()) else {
            continue;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (pixel_width, pixel_height).hash(&mut hasher);
        image.get_raw_image_data().unwrap_or_default().hash(&mut hasher);
        rasters.push(PdfImage {
            index: index as u32,
            x: bounds.left().value,
            y: bounds.bottom().value,
            width: bounds.width().value,
            height: bounds.height().value,
            pixel_width: pixel_width.max(0) as u32,
            pixel_height: pixel_height.max(0) as u32,
            fingerprint: hasher.finish(),
        });
    }
    let rects: Vec<(u32, Rect)> = rasters.iter().map(|i| (i.index, image_rect(i))).collect();
    for group in tile_groups(&rects) {
        if group.len() == 1 || !upright {
            images.extend(group.iter().map(|&i| rasters[i].clone()));
            continue;
        }
        let Some(bounds) = group.iter().map(|&i| rects[i].1).reduce(Rect::union) else {
            continue;
        };
        let scale = render_scale(bounds.width());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for &i in &group {
            rasters[i].fingerprint.hash(&mut hasher);
        }
        images.push(PdfImage {
            index: group.iter().map(|&i| rasters[i].index).min().unwrap_or(0),
            x: bounds.left,
            y: bounds.bottom,
            width: bounds.width(),
            height: bounds.height(),
            pixel_width: (bounds.width() * scale) as u32,
            pixel_height: (bounds.height() * scale) as u32,
            fingerprint: hasher.finish(),
        });
    }
    images.sort_by_key(|i| i.index);
    images
}

fn image_rect(image: &PdfImage) -> Rect {
    Rect {
        left: image.x,
        bottom: image.y,
        right: image.x + image.width,
        top: image.y + image.height,
    }
}

/// Object indices and page rectangles of the raster images on a page.
fn raster_rects(page: &PdfPage<'_>) -> Vec<(u32, Rect)> {
    page.objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.as_image_object().is_some())
        .filter_map(|(index, o)| {
            let b = o.bounds().ok()?;
            Some((
                index as u32,
                Rect {
                    left: b.left().value,
                    bottom: b.bottom().value,
                    right: b.right().value,
                    top: b.top().value,
                },
            ))
        })
        .collect()
}

/// Groups of rectangles that touch or overlap (within a point and a half,
/// as strips are placed with rounding), each group in its input order.
fn tile_groups(rects: &[(u32, Rect)]) -> Vec<Vec<usize>> {
    let slack = 1.5;
    let mut parent: Vec<usize> = (0..rects.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            let (a, b) = (rects[i].1, rects[j].1);
            let touches = a.left - slack < b.right
                && b.left - slack < a.right
                && a.bottom - slack < b.top
                && b.bottom - slack < a.top;
            if touches {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                parent[rj] = ri;
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut slot: HashMap<usize, usize> = HashMap::new();
    for i in 0..rects.len() {
        let r = root(&mut parent, i);
        let at = *slot.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[at].push(i);
    }
    groups
}

/// What a form XObject draws, counted through nested forms.
#[derive(Default, Hash)]
struct FormContents {
    images: usize,
    paths: usize,
    texts: usize,
    /// Size of the first image's data, to tell apart forms of equal shape.
    first_image_bytes: usize,
}

impl FormContents {
    fn add(&mut self, form: &PdfPageXObjectFormObject<'_>) {
        for object in form.iter() {
            match object.object_type() {
                PdfPageObjectType::Image => {
                    if self.images == 0
                        && let Some(image) = object.as_image_object()
                    {
                        self.first_image_bytes =
                            image.get_raw_image_data().map_or(0, |d| d.len());
                    }
                    self.images += 1;
                }
                PdfPageObjectType::Path => self.paths += 1,
                PdfPageObjectType::Text => self.texts += 1,
                PdfPageObjectType::XObjectForm => {
                    if let Some(inner) = object.as_x_object_form_object() {
                        self.add(inner);
                    }
                }
                _ => {}
            }
        }
    }

    /// A picture or a drawing rather than a block of text or a few rules.
    fn is_graphic(&self) -> bool {
        self.images > 0 || self.paths >= 10
    }
}

/// The font size most characters on the page are set in.
fn body_size(lines: &[PdfLine]) -> f32 {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for line in lines {
        *counts.entry((line.font_size * 2.0).round() as u32).or_default() +=
            line.text.chars().count();
    }
    counts
        .into_iter()
        .max_by_key(|&(size, n)| (n, size))
        .map_or(0.0, |(size, _)| size as f32 / 2.0)
}

/// A rectangle on the page, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
}

impl Rect {
    fn width(&self) -> f32 {
        self.right - self.left
    }

    fn height(&self) -> f32 {
        self.top - self.bottom
    }

    fn area(&self) -> f32 {
        self.width().max(0.0) * self.height().max(0.0)
    }

    fn union(self, other: Rect) -> Rect {
        Rect {
            left: self.left.min(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.max(other.right),
            top: self.top.max(other.top),
        }
    }

    fn intersection(self, other: Rect) -> Rect {
        Rect {
            left: self.left.max(other.left),
            bottom: self.bottom.max(other.bottom),
            right: self.right.min(other.right),
            top: self.top.min(other.top),
        }
    }
}

/// An affine transform `[a b c d e f]` as in a PDF `cm` operator.
#[derive(Clone, Copy)]
struct Affine([f32; 6]);

impl Affine {
    fn of(m: &PdfMatrix) -> Affine {
        Affine([m.a(), m.b(), m.c(), m.d(), m.e(), m.f()])
    }

    /// `inner` first, then `self`.
    fn then_inside(self, inner: Affine) -> Affine {
        let [a, b, c, d, e, f] = inner.0;
        let [a2, b2, c2, d2, e2, f2] = self.0;
        Affine([
            a * a2 + b * c2,
            a * b2 + b * d2,
            c * a2 + d * c2,
            c * b2 + d * d2,
            e * a2 + f * c2 + e2,
            e * b2 + f * d2 + f2,
        ])
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    fn rect(&self, left: f32, bottom: f32, right: f32, top: f32) -> Rect {
        let corners = [
            self.apply(left, bottom),
            self.apply(right, bottom),
            self.apply(left, top),
            self.apply(right, top),
        ];
        let xs = corners.map(|c| c.0);
        let ys = corners.map(|c| c.1);
        Rect {
            left: xs.iter().copied().fold(f32::MAX, f32::min),
            bottom: ys.iter().copied().fold(f32::MAX, f32::min),
            right: xs.iter().copied().fold(f32::MIN, f32::max),
            top: ys.iter().copied().fold(f32::MIN, f32::max),
        }
    }
}

/// Where an included graphic actually draws on the page. A form's own
/// bounds often take in an invisible canvas larger than the figure, or
/// stray marks far off the page, so this unions what it draws instead:
/// everything on the page except rectangles spanning the whole canvas.
/// Page text the form did not draw (the caption, the body above) trims
/// the edges; a form mostly under page text is a background, not a figure.
fn figure_bounds(
    form: &PdfPageXObjectFormObject<'_>,
    page: &PdfPage<'_>,
    lines: &[PdfLine],
) -> Option<Rect> {
    let page_rect = Rect {
        left: 0.0,
        bottom: 0.0,
        right: page.width().value,
        top: page.height().value,
    };
    let to_page = Affine::of(&form.matrix().ok()?);
    let mut marks = Vec::new();
    collect_marks(form, to_page, page_rect, &mut marks);
    let extent = marks.iter().map(|m| m.0).reduce(Rect::union)?;
    let bounds = marks
        .iter()
        .filter(|(rect, is_path)| !(*is_path && rect.area() >= extent.area() * 0.8))
        .map(|m| m.0)
        .reduce(Rect::union)?;
    // Glyph boxes run tight; a little room keeps label edges whole.
    let pad = 3.0;
    let bounds = Rect {
        left: bounds.left - pad,
        bottom: bounds.bottom - pad,
        right: bounds.right + pad,
        top: bounds.top + pad,
    }
    .intersection(page_rect);
    let drawn = bounds;
    let mut bounds = bounds;
    // Small type over a figure (labels laid on it as page text) is part of
    // the picture; text at about body size is not.
    let body = body_size(lines);
    for line in lines.iter().filter(|l| l.font_size >= body * 0.8) {
        let rect = Rect {
            left: line.x,
            bottom: line.y,
            right: line.x + line.width,
            top: line.y + line.height,
        };
        if rect.intersection(bounds).area() <= 0.0 {
            continue;
        }
        let (cx, cy) = (line.x + line.width / 2.0, line.y + line.height / 2.0);
        let own = marks.iter().any(|(m, is_path)| {
            !is_path && cx >= m.left && cx <= m.right && cy >= m.bottom && cy <= m.top
        });
        if own {
            continue;
        }
        if cy < (bounds.bottom + bounds.top) / 2.0 {
            bounds.bottom = bounds.bottom.max(rect.top);
        } else {
            bounds.top = bounds.top.min(rect.bottom);
        }
    }
    (bounds.width() > 0.0 && bounds.height() > 0.0 && bounds.area() >= drawn.area() * 0.3)
        .then_some(bounds)
}

/// Page rectangles of what a form draws (and whether each is a path),
/// leaving out marks mostly off the page.
fn collect_marks(
    form: &PdfPageXObjectFormObject<'_>,
    to_page: Affine,
    page_rect: Rect,
    marks: &mut Vec<(Rect, bool)>,
) {
    for object in form.iter() {
        if let Some(inner) = object.as_x_object_form_object() {
            if let Ok(m) = inner.matrix() {
                collect_marks(inner, to_page.then_inside(Affine::of(&m)), page_rect, marks);
            }
            continue;
        }
        let Ok(b) = object.bounds() else { continue };
        let rect = to_page.rect(b.left().value, b.bottom().value, b.right().value, b.top().value);
        let on_page = rect.intersection(page_rect).area();
        if rect.area() <= 0.0 || on_page < rect.area() * 0.5 {
            continue;
        }
        marks.push((rect, object.object_type() == PdfPageObjectType::Path));
    }
}

/// Pixels per point for rendering a region `width` points wide: sharp on a
/// Retina screen without making large figures huge.
fn render_scale(width: f32) -> f32 {
    (1600.0 / width.max(1.0)).clamp(2.0, 4.0)
}

/// The whole page at `scale` pixels per point.
fn render_page<'a>(page: &'a PdfPage<'a>, scale: f32) -> Option<PdfBitmap<'a>> {
    // Pixels stay in pdfium's native BGRA, as image bitmaps come.
    let config = PdfRenderConfig::new()
        .scale_page_by_factor(scale)
        .render_form_data(false)
        .set_reverse_byte_order(false);
    page.render_with_config(&config).ok()
}

/// The area `bounds` of an upright page, cut from `bitmap` (the page
/// rendered at `scale`) and encoded as PNG.
fn crop_region(page: &PdfPage<'_>, bitmap: &PdfBitmap<'_>, scale: f32, bounds: Rect) -> Option<Vec<u8>> {
    let page_height = page.height().value;
    let px = |v: f32| (v * scale).round() as i32;
    let left = px(bounds.left).clamp(0, bitmap.width());
    let top = px(page_height - bounds.top).clamp(0, bitmap.height());
    let right = px(bounds.right).clamp(0, bitmap.width());
    let bottom = px(page_height - bounds.bottom).clamp(0, bitmap.height());
    encode_png(bitmap, (left, top, right - left, bottom - top))
}

/// The stored bytes when the image is a plain RGB or grayscale JPEG.
fn plain_jpeg(image: &PdfPageImageObject<'_>) -> Option<Vec<u8>> {
    let filters: Vec<String> = image.filters().iter().map(|f| f.name().to_string()).collect();
    if filters != ["DCTDecode"] {
        return None;
    }
    // CMYK JPEGs from print workflows come out inverted in most viewers;
    // an ICC-based one is RGB or gray only at 24 or 8 bits per pixel.
    let rgb_or_gray = match image.color_space().ok()? {
        PdfColorSpace::DeviceRGB | PdfColorSpace::DeviceGray => true,
        PdfColorSpace::CalibratedICCProfile => {
            matches!(image.bits_per_pixel().ok()?, 8 | 24)
        }
        _ => false,
    };
    if !rgb_or_gray {
        return None;
    }
    let data = image.get_raw_image_data().ok()?;
    data.starts_with(&[0xFF, 0xD8, 0xFF]).then_some(data)
}

/// The `(left, top, width, height)` part of `bitmap` as PNG.
fn encode_png(bitmap: &PdfBitmap<'_>, crop: (i32, i32, i32, i32)) -> Option<Vec<u8>> {
    let (left, top) = (usize::try_from(crop.0).ok()?, usize::try_from(crop.1).ok()?);
    let width = usize::try_from(crop.2).ok()?;
    let height = usize::try_from(crop.3).ok()?;
    let full_height = usize::try_from(bitmap.height()).ok()?;
    if width == 0 || height == 0 || full_height == 0 {
        return None;
    }
    let raw = bitmap.as_raw_bytes();
    let stride = raw.len() / full_height;
    let (channels, color) = match bitmap.format().ok()? {
        PdfBitmapFormat::Gray => (1, png::ColorType::Grayscale),
        PdfBitmapFormat::BGR => (3, png::ColorType::Rgb),
        PdfBitmapFormat::BGRx => (3, png::ColorType::Rgb),
        PdfBitmapFormat::BGRA => (4, png::ColorType::Rgba),
    };
    let source_channels = match bitmap.format().ok()? {
        PdfBitmapFormat::Gray => 1,
        PdfBitmapFormat::BGR => 3,
        _ => 4,
    };
    if stride < (left + width) * source_channels || full_height < top + height {
        return None;
    }
    let mut pixels = Vec::with_capacity(width * height * channels);
    for row in raw.chunks(stride).skip(top).take(height) {
        let row = &row[left * source_channels..(left + width) * source_channels];
        for px in row.chunks(source_channels) {
            match channels {
                1 => pixels.push(px[0]),
                3 => pixels.extend_from_slice(&[px[2], px[1], px[0]]),
                _ => pixels.extend_from_slice(&[px[2], px[1], px[0], px[3]]),
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&pixels).ok()?;
    }
    Some(out)
}

/// One character with what the line grouping needs.
struct Glyph {
    ch: char,
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    baseline: f32,
    font_size: f32,
    bold: bool,
}

#[derive(Default)]
struct LineBuilder {
    lines: Vec<PdfLine>,
    current: Vec<Glyph>,
}

impl LineBuilder {
    fn run(mut self, text: &PdfPageText<'_>) -> Vec<PdfLine> {
        let chars = text.chars();
        for c in chars.iter() {
            let Some(mut ch) = c.unicode_char() else { continue };
            if ch == '\r' || ch == '\n' {
                // pdfium marks line breaks it inferred; geometry decides
                // for us, but a break is still a hard boundary.
                self.flush();
                continue;
            }
            if ch.is_control() {
                // Fonts without a Unicode map (TeX's Type 1 fonts) give
                // hyphens a control code; pdfium still knows what they are.
                if c.is_hyphen().unwrap_or(false) {
                    ch = '-';
                } else {
                    continue;
                }
            }
            if is_diagonal(&c) {
                continue;
            }
            let Ok(bounds) = c.loose_bounds() else { continue };
            let Ok((_, baseline)) = c.origin() else { continue };
            let font_size = c.scaled_font_size().value.abs();
            if font_size <= 0.0 {
                continue;
            }
            let glyph = Glyph {
                ch,
                left: bounds.left().value,
                right: bounds.right().value,
                bottom: bounds.bottom().value,
                top: bounds.top().value,
                baseline: baseline.value,
                font_size,
                bold: is_bold(&c),
            };
            if !self.continues(&glyph) {
                self.flush();
            }
            self.current.push(glyph);
        }
        self.flush();
        self.lines
    }

    /// Whether `glyph` sits on the line being built: same baseline, and
    /// neither far to the right (another column) nor back to the left.
    fn continues(&self, glyph: &Glyph) -> bool {
        let Some(last) = self.current.last() else {
            return true;
        };
        let size = last.font_size.max(glyph.font_size).max(1.0);
        if (glyph.baseline - last.baseline).abs() > size * 0.5 {
            return false;
        }
        let gap = glyph.left - last.right;
        gap > -size && gap < size * 3.0
    }

    fn flush(&mut self) {
        let glyphs = std::mem::take(&mut self.current);
        if let Some(line) = build_line(glyphs) {
            self.lines.push(line);
        }
    }
}

fn build_line(glyphs: Vec<Glyph>) -> Option<PdfLine> {
    // Words are separated by what is visible: a space glyph counts only if
    // the next glyph does not sit back on top of it (troff kerns that way),
    // and a clear gap counts even without one.
    let mut text = String::new();
    let mut prev: Option<&Glyph> = None;
    let mut space_glyph = false;
    for g in &glyphs {
        if g.ch.is_whitespace() {
            space_glyph = true;
            continue;
        }
        if let Some(p) = prev {
            let gap = g.left - p.right;
            let size = g.font_size.min(p.font_size);
            if (space_glyph && gap > size * 0.1) || gap > size * 0.25 {
                text.push(' ');
            }
        }
        text.push(g.ch);
        prev = Some(g);
        space_glyph = false;
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    let visible: Vec<&Glyph> = glyphs.iter().filter(|g| !g.ch.is_whitespace()).collect();
    let left = visible.iter().map(|g| g.left).fold(f32::MAX, f32::min);
    let right = visible.iter().map(|g| g.right).fold(f32::MIN, f32::max);
    let bottom = visible.iter().map(|g| g.bottom).fold(f32::MAX, f32::min);
    let top = visible.iter().map(|g| g.top).fold(f32::MIN, f32::max);
    let font_size = dominant_size(&visible);
    let bold_count = visible.iter().filter(|g| g.bold).count();
    Some(PdfLine {
        text,
        x: left,
        y: bottom,
        width: right - left,
        height: top - bottom,
        font_size,
        bold: bold_count * 2 > visible.len(),
    })
}

/// The font size most characters are set in, to a tenth of a point.
fn dominant_size(glyphs: &[&Glyph]) -> f32 {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for g in glyphs {
        *counts.entry((g.font_size * 10.0).round() as u32).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(size, n)| (n, size))
        .map(|(size, _)| size as f32 / 10.0)
        .unwrap_or(0.0)
}

/// Text set at a slant ("DRAFT", "CONFIDENTIAL" across the page) is a
/// watermark. Upright and vertical text (rotated by a right angle) stays.
fn is_diagonal(c: &PdfPageTextChar<'_>) -> bool {
    let Ok(angle) = c.angle_degrees() else {
        return false;
    };
    let off_axis = angle.rem_euclid(90.0);
    off_axis > 10.0 && off_axis < 80.0
}

/// Whether a character is set in a bold face. The font name is trusted
/// first: pdfium's weight for fonts without a descriptor (the standard
/// 14, as troff and old tools emit them) is a guess that is often wrong.
fn is_bold(c: &PdfPageTextChar<'_>) -> bool {
    let name = c.font_name().to_ascii_lowercase();
    if ["bold", "black", "heavy", "semibold"].iter().any(|w| name.contains(w)) {
        return true;
    }
    if ["roman", "regular", "light", "book", "medium", "italic", "oblique"]
        .iter()
        .any(|w| name.contains(w))
    {
        return false;
    }
    match c.font_weight() {
        Some(PdfFontWeight::Weight600)
        | Some(PdfFontWeight::Weight700Bold)
        | Some(PdfFontWeight::Weight800)
        | Some(PdfFontWeight::Weight900) => true,
        Some(PdfFontWeight::Custom(w)) => w >= 600,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page Letter PDF with Helvetica (F1) and Helvetica-Bold (F2)
    /// and the given content stream.
    fn pdf(content: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> /XObject << /Im1 8 0 R /Fm1 9 0 R >> >> /Contents 6 0 R >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
            "<< /Title (Paper) /Author (Ada) >>".to_string(),
            // A 4x4 RGB image: red top half, blue bottom half.
            {
                let red = "FF0000".repeat(8);
                let blue = "0000FF".repeat(8);
                let data = format!("{red}{blue}>");
                format!(
                    "<< /Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length {} >>\nstream\n{data}\nendstream",
                    data.len() + 1
                )
            },
            // An included graphic: the image above, drawn in a unit box.
            {
                let draw = "/Im1 Do";
                format!(
                    "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im1 8 0 R >> >> /Length {} >>\nstream\n{draw}\nendstream",
                    draw.len() + 1
                )
            },
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Info 7 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn lines_come_with_geometry_sizes_and_weights() {
        let content = "\
BT /F1 9 Tf 72 760 Td (Journal of Things) Tj ET
BT /F2 16 Tf 72 700 Td (A Title) Tj ET
BT /F1 10 Tf 72 680 Td (Left column line one) Tj 0 -12 Td (left two) Tj ET
BT /F1 10 Tf 320 680 Td (Right column line one) Tj 0 -12 Td (right two) Tj ET
BT /F1 9 Tf 300 40 Td (1) Tj ET";
        let dir = tempdir();
        let path = dir.join("sample.pdf");
        std::fs::write(&path, pdf(content)).unwrap();

        let text = PdfiumReader::new().read(&path).expect("pdfium reads the file");
        assert_eq!(text.title.as_deref(), Some("Paper"));
        assert_eq!(text.author.as_deref(), Some("Ada"));
        assert_eq!(text.pages.len(), 1);
        let page = &text.pages[0];
        assert_eq!((page.width, page.height), (612.0, 792.0));

        let texts: Vec<&str> = page.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Journal of Things",
                "A Title",
                "Left column line one",
                "left two",
                "Right column line one",
                "right two",
                "1",
            ]
        );
        let title = &page.lines[1];
        assert!((title.font_size - 16.0).abs() < 0.1, "{}", title.font_size);
        assert!(title.bold);
        assert!(!page.lines[0].bold);
        assert!((page.lines[2].font_size - 10.0).abs() < 0.1);
        assert!((page.lines[2].x - 72.0).abs() < 1.0);
        assert!((page.lines[4].x - 320.0).abs() < 1.0);
        assert!(page.lines[2].y > page.lines[3].y);
        assert!(page.lines[0].y > title.y && title.y > page.lines[2].y);
        assert!(page.lines[6].y < 50.0);
        assert!(page.lines[2].height > 8.0 && page.lines[2].height < 16.0);
        assert!(page.lines[2].width > 60.0);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn images_are_listed_where_they_sit_and_decode_to_png() {
        let content = "\
BT /F1 10 Tf 72 700 Td (Body text) Tj ET
q 200 0 0 100 72 400 cm /Im1 Do Q
q 50 0 0 50 300 100 cm /Im1 Do Q
BT /F1 40 Tf 0.7071 0.7071 -0.7071 0.7071 150 300 Tm (DRAFT) Tj ET";
        let dir = tempdir();
        let path = dir.join("images.pdf");
        std::fs::write(&path, pdf(content)).unwrap();

        let text = PdfiumReader::new().read(&path).unwrap();
        let page = &text.pages[0];
        let texts: Vec<&str> = page.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Body text"], "the slanted watermark is dropped");
        assert_eq!(page.images.len(), 2);
        let first = &page.images[0];
        assert!((first.x - 72.0).abs() < 0.5 && (first.y - 400.0).abs() < 0.5);
        assert!((first.width - 200.0).abs() < 0.5 && (first.height - 100.0).abs() < 0.5);
        assert_eq!((first.pixel_width, first.pixel_height), (4, 4));
        assert_eq!(
            first.fingerprint, page.images[1].fingerprint,
            "the same image drawn twice is recognisable"
        );

        let (bytes, mime) = PdfiumReader::new()
            .image(&path, 0, first.index)
            .expect("the image decodes");
        assert_eq!(mime, "image/png");
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert!(info.width >= 4 && info.height >= 4);
        let px = |x: u32, y: u32| {
            let channels = info.line_size / info.width as usize;
            let at = y as usize * info.line_size + x as usize * channels;
            (buf[at], buf[at + 1], buf[at + 2])
        };
        assert_eq!(px(0, 0), (255, 0, 0), "top is red");
        assert_eq!(px(0, info.height - 1), (0, 0, 255), "bottom is blue");

        assert!(PdfiumReader::new().image(&path, 0, 0).is_none(), "object 0 is text");
        assert!(PdfiumReader::new().image(&path, 3, 1).is_none(), "no page 3");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn included_graphics_are_figures_rendered_from_the_page() {
        let content = "\
BT /F1 10 Tf 72 700 Td (Body text) Tj ET
q 100 0 0 80 300 500 cm /Fm1 Do Q";
        let dir = tempdir();
        let path = dir.join("form.pdf");
        std::fs::write(&path, pdf(content)).unwrap();

        let text = PdfiumReader::new().read(&path).unwrap();
        let figures = &text.pages[0].images;
        assert_eq!(figures.len(), 1, "{figures:?}");
        let figure = &figures[0];
        // The drawn area, padded by 3 points.
        assert!((figure.x - 297.0).abs() < 0.5 && (figure.y - 497.0).abs() < 0.5);
        assert!((figure.width - 106.0).abs() < 0.5 && (figure.height - 86.0).abs() < 0.5);

        let (bytes, mime) = PdfiumReader::new().image(&path, 0, figure.index).unwrap();
        assert_eq!(mime, "image/png");
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (424, 344), "106x86 pt at 4 px/pt");
        let channels = info.line_size / info.width as usize;
        let px = |x: usize, y: usize| {
            let at = y * info.line_size + x * channels;
            (buf[at], buf[at + 1], buf[at + 2])
        };
        assert_eq!(px(212, 52), (255, 0, 0), "the top of the figure is the red half");
        assert_eq!(px(212, 292), (0, 0, 255), "the bottom is the blue half");
        assert_eq!(px(4, 4), (255, 255, 255), "the padding is the white page");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn strips_and_stacked_layers_make_one_picture_as_it_shows() {
        // Two strips of one picture, a layer drawn over both (an animation
        // step), and a separate image elsewhere on the page.
        let content = "\
q 200 0 0 50 72 500 cm /Im1 Do Q
q 200 0 0 50 72 450 cm /Im1 Do Q
q 200 0 0 100 72 450 cm /Im1 Do Q
q 60 0 0 60 400 100 cm /Im1 Do Q";
        let dir = tempdir();
        let path = dir.join("strips.pdf");
        std::fs::write(&path, pdf(content)).unwrap();

        let text = PdfiumReader::new().read(&path).unwrap();
        let images = &text.pages[0].images;
        assert_eq!(images.len(), 2, "{images:?}");
        let picture = &images[0];
        assert!((picture.x - 72.0).abs() < 0.5 && (picture.y - 450.0).abs() < 0.5);
        assert!((picture.width - 200.0).abs() < 0.5 && (picture.height - 100.0).abs() < 0.5);
        assert!((images[1].x - 400.0).abs() < 0.5, "the separate image stays apart");

        let decode = |index: u32| {
            let (bytes, mime) = PdfiumReader::new().image(&path, 0, index).unwrap();
            assert_eq!(mime, "image/png");
            let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
            let mut buf = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut buf).unwrap();
            (info, buf)
        };
        let (info, buf) = decode(picture.index);
        assert_eq!((info.width, info.height), (800, 400), "200x100 pt at 4 px/pt");
        let channels = info.line_size / info.width as usize;
        let px = |x: usize, y: usize| {
            let at = y * info.line_size + x * channels;
            (buf[at], buf[at + 1], buf[at + 2])
        };
        // The top layer shows: red over its upper half, blue below. Each
        // strip alone would have been red-over-blue within its own height.
        assert_eq!(px(400, 100), (255, 0, 0));
        assert_eq!(px(400, 150), (255, 0, 0));
        assert_eq!(px(400, 300), (0, 0, 255));
        let (other, _) = decode(picture.index + 1);
        assert_eq!((other.width, other.height), (800, 400), "any strip gives the whole");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_batch_gives_the_same_pictures_as_single_requests() {
        // A figure rendered from the page, a plain raster, and two strips.
        let content = "\
BT /F1 10 Tf 72 700 Td (Body text) Tj ET
q 100 0 0 80 300 500 cm /Fm1 Do Q
q 60 0 0 60 400 100 cm /Im1 Do Q
q 200 0 0 50 72 300 cm /Im1 Do Q
q 200 0 0 50 72 250 cm /Im1 Do Q";
        let dir = tempdir();
        let path = dir.join("batch.pdf");
        std::fs::write(&path, pdf(content)).unwrap();

        let reader = PdfiumReader::new();
        let images = reader.read(&path).unwrap().pages.remove(0).images;
        assert_eq!(images.len(), 3, "{images:?}");
        let mut requests: Vec<(u32, u32)> = images.iter().map(|i| (0, i.index)).collect();
        requests.push((0, 99));
        requests.push((7, 0));
        let batch = reader.read_images(&path, &requests);
        assert_eq!(batch.len(), requests.len());
        for (request, picture) in requests.iter().zip(&batch) {
            assert_eq!(*picture, reader.read_image(&path, request.0, request.1), "{request:?}");
        }
        assert!(batch[..3].iter().all(Option::is_some));
        assert!(batch[3..].iter().all(Option::is_none), "unknown objects and pages give nothing");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_files_give_none() {
        let dir = tempdir();
        let path = dir.join("not.pdf");
        std::fs::write(&path, b"hello").unwrap();
        assert!(PdfiumReader::new().read(&path).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eddy-pdf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
