//! Map rendering from vector tiles into a raster image, using Skia.

use std::{collections::HashSet, sync::LazyLock};

use fast_mvt::{MvtFeatureRef, MvtGeometry, MvtReaderRef, MvtValueRef};
use skia_safe::{
    AlphaType, Canvas, Color, ColorType, Font, FontMgr, FontStyle, ImageInfo, Paint, Path,
    PathBuilder, PathFillType, Pixmap, Rect, Typeface, dash_path_effect, jpeg_encoder,
    paint::{Cap as LineCap, Join as LineJoin, Style as PaintStyle},
    surfaces,
};

use super::{
    DEVICE_PIXEL_RATIO, FRAME_HEIGHT, FRAME_WIDTH, MapError, MapResult, RenderTile, TILE_SIZE,
    TilePlacement, Viewport, mercator_tile_position, source_zoom,
};

static MAP_TYPEFACES: LazyLock<Option<MapTypefaces>> = LazyLock::new(load_map_typefaces);

#[derive(Clone, Copy)]
enum MapTheme {
    Warm,
    Verdant,
    Night,
}

impl MapTheme {
    fn parse(value: &str) -> Self {
        match value {
            "verdant" => Self::Verdant,
            "night" => Self::Night,
            _ => Self::Warm,
        }
    }

    fn palette(self) -> Palette {
        match self {
            Self::Warm => Palette {
                background: [234, 240, 223, 255],
                residential: [238, 238, 228, 255],
                wood: [181, 215, 178, 255],
                grass: [202, 226, 191, 255],
                sand: [240, 228, 190, 255],
                park: [183, 221, 183, 255],
                water: [145, 204, 224, 255],
                waterway: [91, 169, 199, 255],
                aeroway: [224, 226, 220, 255],
                building: [213, 211, 202, 255],
                building_outline: [190, 188, 179, 255],
                road_casing: [167, 181, 198, 255],
                road_minor: [249, 249, 242, 255],
                road_major: [227, 233, 239, 255],
                motorway: [214, 225, 235, 255],
                rail: [130, 137, 143, 255],
                boundary_country_land: [105, 118, 126, 225],
                boundary_country_sea: [96, 110, 118, 170],
                boundary_subdivision: [137, 155, 184, 220],
                label_country: [72, 82, 75, 255],
                label_city: [52, 57, 53, 255],
                label_secondary: [92, 97, 89, 245],
                label_halo: [246, 247, 239, 238],
                marker_neutral: [86, 100, 90, 255],
                marker_transport: [77, 111, 139, 255],
                marker_health: [159, 88, 84, 255],
            },
            Self::Verdant => Palette {
                background: [222, 234, 210, 255],
                residential: [233, 236, 222, 255],
                wood: [154, 203, 157, 255],
                grass: [187, 220, 174, 255],
                sand: [235, 224, 183, 255],
                park: [165, 214, 167, 255],
                water: [125, 197, 219, 255],
                waterway: [71, 156, 190, 255],
                aeroway: [218, 224, 214, 255],
                building: [207, 208, 194, 255],
                building_outline: [180, 185, 170, 255],
                road_casing: [152, 167, 172, 255],
                road_minor: [248, 248, 235, 255],
                road_major: [231, 235, 221, 255],
                motorway: [220, 230, 218, 255],
                rail: [114, 126, 124, 255],
                boundary_country_land: [87, 108, 103, 225],
                boundary_country_sea: [78, 117, 128, 170],
                boundary_subdivision: [123, 148, 155, 215],
                label_country: [53, 74, 62, 255],
                label_city: [39, 55, 46, 255],
                label_secondary: [69, 89, 76, 245],
                label_halo: [241, 246, 230, 238],
                marker_neutral: [64, 108, 78, 255],
                marker_transport: [53, 113, 146, 255],
                marker_health: [154, 78, 75, 255],
            },
            Self::Night => Palette {
                background: [24, 27, 27, 255],
                residential: [38, 42, 41, 255],
                wood: [31, 58, 47, 255],
                grass: [47, 66, 50, 255],
                sand: [83, 73, 47, 255],
                park: [39, 73, 54, 255],
                water: [29, 63, 82, 255],
                waterway: [46, 108, 137, 255],
                aeroway: [52, 54, 53, 255],
                building: [66, 67, 64, 255],
                building_outline: [82, 82, 77, 255],
                road_casing: [25, 27, 27, 255],
                road_minor: [97, 99, 95, 255],
                road_major: [119, 136, 149, 255],
                motorway: [145, 164, 178, 255],
                rail: [116, 116, 111, 255],
                boundary_country_land: [151, 164, 172, 225],
                boundary_country_sea: [135, 151, 160, 170],
                boundary_subdivision: [103, 121, 148, 215],
                label_country: [205, 217, 211, 255],
                label_city: [235, 237, 232, 255],
                label_secondary: [185, 193, 186, 245],
                label_halo: [24, 27, 27, 238],
                marker_neutral: [164, 184, 170, 255],
                marker_transport: [135, 174, 204, 255],
                marker_health: [215, 139, 135, 255],
            },
        }
    }
}

#[derive(Clone, Copy)]
enum RoadEmphasis {
    Off,
    Quiet,
    Standard,
    Bold,
}

impl RoadEmphasis {
    fn parse(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "quiet" => Self::Quiet,
            "bold" => Self::Bold,
            _ => Self::Standard,
        }
    }

    fn width_scale(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Quiet => 0.68,
            Self::Standard => 1.0,
            Self::Bold => 1.38,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum AnnotationDensity {
    Off,
    Sparse,
    Standard,
}

impl AnnotationDensity {
    fn parse(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "sparse" => Self::Sparse,
            _ => Self::Standard,
        }
    }

    fn rank_scale(self) -> f64 {
        match self {
            Self::Off => 0.0,
            Self::Sparse => 0.48,
            Self::Standard => 1.0,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct MapStyle {
    theme: MapTheme,
    roads: RoadEmphasis,
    annotations: AnnotationDensity,
    terrain: bool,
    buildings: bool,
    boundaries: bool,
}

impl MapStyle {
    pub(super) fn parse(
        theme: &str,
        roads: &str,
        annotations: &str,
        terrain: bool,
        buildings: bool,
        boundaries: bool,
    ) -> Self {
        Self {
            theme: MapTheme::parse(theme),
            roads: RoadEmphasis::parse(roads),
            annotations: AnnotationDensity::parse(annotations),
            terrain,
            buildings,
            boundaries,
        }
    }
}

#[derive(Clone, Copy)]
struct Palette {
    background: [u8; 4],
    residential: [u8; 4],
    wood: [u8; 4],
    grass: [u8; 4],
    sand: [u8; 4],
    park: [u8; 4],
    water: [u8; 4],
    waterway: [u8; 4],
    aeroway: [u8; 4],
    building: [u8; 4],
    building_outline: [u8; 4],
    road_casing: [u8; 4],
    road_minor: [u8; 4],
    road_major: [u8; 4],
    motorway: [u8; 4],
    rail: [u8; 4],
    boundary_country_land: [u8; 4],
    boundary_country_sea: [u8; 4],
    boundary_subdivision: [u8; 4],
    label_country: [u8; 4],
    label_city: [u8; 4],
    label_secondary: [u8; 4],
    label_halo: [u8; 4],
    marker_neutral: [u8; 4],
    marker_transport: [u8; 4],
    marker_health: [u8; 4],
}

pub(super) fn render_jpeg(
    viewport: Viewport,
    style: MapStyle,
    tiles: &[RenderTile],
) -> MapResult<Vec<u8>> {
    let mut pixels = render_vector_tiles(viewport, style, tiles)?;
    encode_jpeg(&mut pixels)
}

fn encode_jpeg(pixels: &mut [u8]) -> MapResult<Vec<u8>> {
    let image_info = frame_image_info();
    let row_bytes = FRAME_WIDTH as usize * 4;
    let pixmap = Pixmap::new(&image_info, pixels, row_bytes)
        .ok_or_else(|| MapError::new("could not expose map pixels to the JPEG encoder"))?;
    let options = jpeg_encoder::Options {
        quality: 82,
        ..jpeg_encoder::Options::default()
    };
    jpeg_encoder::encode_pixmap(&pixmap, &options)
        .map(|data| data.as_bytes().to_vec())
        .ok_or_else(|| MapError::new("JPEG encoding failed"))
}

struct RenderTransform {
    center_x: f64,
    center_y: f64,
    pixels_per_tile: f64,
}

impl RenderTransform {
    fn new(viewport: Viewport) -> Self {
        let source_zoom = source_zoom(viewport);
        let (center_x, center_y) = mercator_tile_position(viewport, source_zoom);
        let overzoom = 2_f64.powf(viewport.zoom - f64::from(source_zoom));
        Self {
            center_x,
            center_y,
            pixels_per_tile: TILE_SIZE * DEVICE_PIXEL_RATIO * overzoom,
        }
    }

    fn point(
        &self,
        placement: &TilePlacement,
        extent: u32,
        coordinate: fast_mvt::MvtCoord,
    ) -> (f32, f32) {
        let world_x = f64::from(placement.raw_x) + f64::from(coordinate.x) / f64::from(extent);
        let world_y = f64::from(placement.key.y) + f64::from(coordinate.y) / f64::from(extent);
        (
            ((world_x - self.center_x) * self.pixels_per_tile + f64::from(FRAME_WIDTH) / 2.0)
                as f32,
            ((world_y - self.center_y) * self.pixels_per_tile + f64::from(FRAME_HEIGHT) / 2.0)
                as f32,
        )
    }
}

fn render_vector_tiles(
    viewport: Viewport,
    style: MapStyle,
    tiles: &[RenderTile],
) -> MapResult<Vec<u8>> {
    let palette = style.theme.palette();
    let transform = RenderTransform::new(viewport);
    let image_info = frame_image_info();
    let row_bytes = FRAME_WIDTH as usize * 4;
    let mut pixels = vec![0; row_bytes * FRAME_HEIGHT as usize];

    {
        let mut surface = surfaces::wrap_pixels(&image_info, &mut pixels, row_bytes, None)
            .ok_or_else(|| MapError::new("could not allocate map frame"))?;
        let canvas = surface.canvas();
        canvas.clear(color(palette.background));

        if style.terrain {
            render_fills(
                canvas,
                tiles,
                "landcover",
                &transform,
                |feature| match string_property(feature, "class") {
                    Some("wood") => Some(palette.wood),
                    Some("grass" | "farmland") => Some(palette.grass),
                    Some("sand") => Some(palette.sand),
                    _ => None,
                },
            )?;
            render_fills(
                canvas,
                tiles,
                "landuse",
                &transform,
                |feature| match string_property(feature, "class") {
                    Some("residential" | "commercial" | "industrial") => Some(palette.residential),
                    Some("cemetery" | "grass" | "pitch" | "school") => Some(palette.grass),
                    _ => None,
                },
            )?;
            render_fills(canvas, tiles, "park", &transform, |_| Some(palette.park))?;
        }
        render_fills(canvas, tiles, "water", &transform, |_| Some(palette.water))?;
        render_fills(canvas, tiles, "aeroway", &transform, |_| {
            Some(palette.aeroway)
        })?;

        render_lines(canvas, tiles, "waterway", &transform, |_| {
            Some(LineStyle::solid(
                palette.waterway,
                1.2 * DEVICE_PIXEL_RATIO as f32,
            ))
        })?;

        if !matches!(style.roads, RoadEmphasis::Off) {
            render_lines(canvas, tiles, "transportation", &transform, |feature| {
                road_style(feature, viewport.zoom, palette, style.roads, true)
            })?;
            render_lines(canvas, tiles, "transportation", &transform, |feature| {
                road_style(feature, viewport.zoom, palette, style.roads, false)
            })?;
        }

        if style.buildings {
            render_fills(canvas, tiles, "building", &transform, |_| {
                Some(palette.building)
            })?;
            render_lines(canvas, tiles, "building", &transform, |_| {
                Some(LineStyle::solid(
                    palette.building_outline,
                    DEVICE_PIXEL_RATIO as f32 * 0.55,
                ))
            })?;
        }

        if style.boundaries {
            render_lines(canvas, tiles, "boundary", &transform, |feature| {
                let admin_level = number_property(feature, "admin_level").unwrap_or(6.0);
                (admin_level > 2.0 && admin_level <= 6.0).then_some(LineStyle::solid(
                    palette.boundary_subdivision,
                    0.85 * DEVICE_PIXEL_RATIO as f32,
                ))
            })?;
            render_lines(canvas, tiles, "boundary", &transform, |feature| {
                let admin_level = number_property(feature, "admin_level").unwrap_or(6.0);
                let maritime = number_property(feature, "maritime").unwrap_or(0.0) != 0.0;
                (admin_level <= 2.0 && maritime).then_some(LineStyle::dashed(
                    palette.boundary_country_sea,
                    1.0 * DEVICE_PIXEL_RATIO as f32,
                    [
                        1.25 * DEVICE_PIXEL_RATIO as f32,
                        3.25 * DEVICE_PIXEL_RATIO as f32,
                    ],
                ))
            })?;
            render_lines(canvas, tiles, "boundary", &transform, |feature| {
                let admin_level = number_property(feature, "admin_level").unwrap_or(6.0);
                let maritime = number_property(feature, "maritime").unwrap_or(0.0) != 0.0;
                (admin_level <= 2.0 && !maritime).then_some(LineStyle::solid(
                    palette.boundary_country_land,
                    1.15 * DEVICE_PIXEL_RATIO as f32,
                ))
            })?;
        }

        render_annotations(
            canvas,
            tiles,
            &transform,
            viewport.zoom,
            palette,
            style.annotations,
        )?;
    }

    Ok(pixels)
}

fn render_fills<F>(
    canvas: &Canvas,
    tiles: &[RenderTile],
    layer_name: &str,
    transform: &RenderTransform,
    mut fill_for: F,
) -> MapResult<()>
where
    F: FnMut(MvtFeatureRef<'_>) -> Option<[u8; 4]>,
{
    for tile in tiles {
        if tile.bytes.is_empty() {
            continue;
        }
        let reader = MvtReaderRef::new(&tile.bytes)
            .map_err(|error| MapError::new(format!("invalid vector tile: {error}")))?;
        let Some(layer) = reader.layers().find(|layer| layer.name() == layer_name) else {
            continue;
        };
        for feature in layer.features() {
            let Some(fill) = fill_for(feature) else {
                continue;
            };
            let geometry = feature
                .geometry()
                .map_err(|error| MapError::new(format!("invalid vector geometry: {error}")))?;
            fill_geometry(
                canvas,
                &geometry,
                &tile.placement,
                layer.extent(),
                transform,
                fill,
            );
        }
    }
    Ok(())
}

fn render_lines<F>(
    canvas: &Canvas,
    tiles: &[RenderTile],
    layer_name: &str,
    transform: &RenderTransform,
    mut stroke_for: F,
) -> MapResult<()>
where
    F: FnMut(MvtFeatureRef<'_>) -> Option<LineStyle>,
{
    for tile in tiles {
        if tile.bytes.is_empty() {
            continue;
        }
        let reader = MvtReaderRef::new(&tile.bytes)
            .map_err(|error| MapError::new(format!("invalid vector tile: {error}")))?;
        let Some(layer) = reader.layers().find(|layer| layer.name() == layer_name) else {
            continue;
        };
        for feature in layer.features() {
            let Some(style) = stroke_for(feature) else {
                continue;
            };
            let geometry = feature
                .geometry()
                .map_err(|error| MapError::new(format!("invalid vector geometry: {error}")))?;
            stroke_geometry(
                canvas,
                &geometry,
                &tile.placement,
                layer.extent(),
                transform,
                style,
            );
        }
    }
    Ok(())
}

fn fill_geometry(
    canvas: &Canvas,
    geometry: &MvtGeometry,
    placement: &TilePlacement,
    extent: u32,
    transform: &RenderTransform,
    fill: [u8; 4],
) {
    let mut builder = PathBuilder::new_with_fill_type(PathFillType::EvenOdd);
    match geometry {
        MvtGeometry::Polygon(polygon) => {
            append_polygon(&mut builder, polygon, placement, extent, transform);
        }
        MvtGeometry::MultiPolygon(polygons) => {
            for polygon in &polygons.0 {
                append_polygon(&mut builder, polygon, placement, extent, transform);
            }
        }
        _ => return,
    }
    if builder.is_empty() {
        return;
    }
    let path = builder.detach();
    let mut paint = Paint::default();
    paint.set_color(color(fill));
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    canvas.draw_path(&path, &paint);
}

fn append_polygon(
    builder: &mut PathBuilder,
    polygon: &fast_mvt::MvtPolygon,
    placement: &TilePlacement,
    extent: u32,
    transform: &RenderTransform,
) {
    append_line(
        builder,
        polygon.exterior(),
        placement,
        extent,
        transform,
        true,
    );
    for ring in polygon.interiors() {
        append_line(builder, ring, placement, extent, transform, true);
    }
}

fn stroke_geometry(
    canvas: &Canvas,
    geometry: &MvtGeometry,
    placement: &TilePlacement,
    extent: u32,
    transform: &RenderTransform,
    style: LineStyle,
) {
    let mut builder = PathBuilder::new();
    match geometry {
        MvtGeometry::LineString(line) => {
            append_line(&mut builder, line, placement, extent, transform, false);
        }
        MvtGeometry::MultiLineString(lines) => {
            for line in &lines.0 {
                append_line(&mut builder, line, placement, extent, transform, false);
            }
        }
        MvtGeometry::Polygon(polygon) => {
            append_polygon(&mut builder, polygon, placement, extent, transform);
        }
        MvtGeometry::MultiPolygon(polygons) => {
            for polygon in &polygons.0 {
                append_polygon(&mut builder, polygon, placement, extent, transform);
            }
        }
        _ => return,
    }
    if builder.is_empty() {
        return;
    }
    let path = builder.detach();
    stroke_path(canvas, &path, style);
}

fn stroke_path(canvas: &Canvas, path: &Path, style: LineStyle) {
    let mut paint = Paint::default();
    paint.set_color(color(style.color));
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(style.width);
    paint.set_stroke_cap(style.line_cap);
    paint.set_stroke_join(LineJoin::Miter);
    paint.set_path_effect(
        style
            .dash
            .and_then(|intervals| dash_path_effect::new(&intervals, 0.0)),
    );
    canvas.draw_path(path, &paint);
}

#[derive(Clone)]
struct MapTypefaces {
    regular: Typeface,
    bold: Typeface,
}

fn load_map_typefaces() -> Option<MapTypefaces> {
    let manager = FontMgr::default();
    let regular = ["Inter", "Helvetica Neue", "Arial", "DejaVu Sans", ""]
        .iter()
        .find_map(|family| manager.match_family_style(family, FontStyle::normal()))?;
    let bold = ["Inter", "Helvetica Neue", "Arial", "DejaVu Sans", ""]
        .iter()
        .find_map(|family| manager.match_family_style(family, FontStyle::bold()))
        .unwrap_or_else(|| regular.clone());
    Some(MapTypefaces { regular, bold })
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum AnnotationKind {
    Country,
    City,
    Town,
    Village,
    Airport,
    Peak,
    Health,
    Education,
    Landmark,
    Transit,
}

impl AnnotationKind {
    fn priority(self) -> u8 {
        match self {
            Self::Country => 0,
            Self::City => 1,
            Self::Airport => 2,
            Self::Town => 3,
            Self::Peak => 4,
            Self::Health => 5,
            Self::Education => 6,
            Self::Landmark => 7,
            Self::Transit => 8,
            Self::Village => 9,
        }
    }

    fn marker(self) -> Option<MarkerKind> {
        match self {
            Self::Airport => Some(MarkerKind::Diamond),
            Self::Peak => Some(MarkerKind::Triangle),
            Self::Health => Some(MarkerKind::Health),
            Self::Education | Self::Landmark => Some(MarkerKind::Circle),
            Self::Transit => Some(MarkerKind::Square),
            Self::Country | Self::City | Self::Town | Self::Village => None,
        }
    }
}

#[derive(Clone, Copy)]
enum MarkerKind {
    Circle,
    Square,
    Diamond,
    Triangle,
    Health,
}

struct Annotation {
    name: String,
    kind: AnnotationKind,
    rank: f64,
    capital: bool,
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct LabelBounds {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl LabelBounds {
    fn intersects(self, other: Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }

    fn inside_frame(self) -> bool {
        self.left >= 0.0
            && self.top >= 0.0
            && self.right <= FRAME_WIDTH as f32
            && self.bottom <= FRAME_HEIGHT as f32
    }
}

fn render_annotations(
    canvas: &Canvas,
    tiles: &[RenderTile],
    transform: &RenderTransform,
    zoom: f64,
    palette: Palette,
    density: AnnotationDensity,
) -> MapResult<()> {
    if density == AnnotationDensity::Off {
        return Ok(());
    }
    let Some(typefaces) = MAP_TYPEFACES.as_ref() else {
        return Ok(());
    };

    let mut annotations = Vec::new();
    let mut seen = HashSet::new();

    for tile in tiles {
        if tile.bytes.is_empty() {
            continue;
        }
        let reader = MvtReaderRef::new(&tile.bytes)
            .map_err(|error| MapError::new(format!("invalid vector tile: {error}")))?;

        if let Some(layer) = reader.layers().find(|layer| layer.name() == "place") {
            for feature in layer.features() {
                let Some(kind) = place_kind(feature, zoom) else {
                    continue;
                };
                let rank = number_property(feature, "rank").unwrap_or(99.0);
                let capital = number_property(feature, "capital").unwrap_or(0.0) == 2.0;
                if !place_rank_visible(kind, rank, capital, zoom, density) {
                    continue;
                }
                push_annotation(
                    &mut annotations,
                    &mut seen,
                    feature,
                    kind,
                    rank,
                    capital,
                    tile,
                    layer.extent(),
                    transform,
                )?;
            }
        }

        if zoom >= 8.0
            && let Some(layer) = reader
                .layers()
                .find(|layer| layer.name() == "aerodrome_label")
        {
            for feature in layer.features() {
                let Some(rank) = aerodrome_rank(feature, zoom) else {
                    continue;
                };
                if density == AnnotationDensity::Sparse && rank > 1.0 {
                    continue;
                }
                push_annotation(
                    &mut annotations,
                    &mut seen,
                    feature,
                    AnnotationKind::Airport,
                    rank,
                    false,
                    tile,
                    layer.extent(),
                    transform,
                )?;
            }
        }

        if zoom >= 9.5
            && let Some(layer) = reader
                .layers()
                .find(|layer| layer.name() == "mountain_peak")
        {
            for feature in layer.features() {
                let rank = number_property(feature, "rank").unwrap_or(99.0);
                if !peak_rank_visible(rank, zoom, density) {
                    continue;
                }
                push_annotation(
                    &mut annotations,
                    &mut seen,
                    feature,
                    AnnotationKind::Peak,
                    rank,
                    false,
                    tile,
                    layer.extent(),
                    transform,
                )?;
            }
        }

        if zoom >= 11.5
            && let Some(layer) = reader.layers().find(|layer| layer.name() == "poi")
        {
            for feature in layer.features() {
                let Some(kind) = poi_kind(feature) else {
                    continue;
                };
                let rank = number_property(feature, "rank").unwrap_or(99.0);
                if !poi_rank_visible(rank, zoom, density) {
                    continue;
                }
                push_annotation(
                    &mut annotations,
                    &mut seen,
                    feature,
                    kind,
                    rank,
                    false,
                    tile,
                    layer.extent(),
                    transform,
                )?;
            }
        }
    }

    annotations.sort_by(|left, right| {
        left.kind
            .priority()
            .cmp(&right.kind.priority())
            .then_with(|| right.capital.cmp(&left.capital))
            .then_with(|| left.rank.total_cmp(&right.rank))
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut occupied = Vec::new();
    for annotation in annotations {
        let text = match annotation.kind {
            AnnotationKind::Country => annotation.name.to_uppercase(),
            _ => annotation.name,
        };
        let logical_size = match (annotation.kind, annotation.capital) {
            (AnnotationKind::Country, _) => 14.0,
            (AnnotationKind::City, true) => 15.0,
            (AnnotationKind::City, false) => 13.0,
            (AnnotationKind::Town, _) => 11.5,
            (AnnotationKind::Village, _) => 10.5,
            (AnnotationKind::Airport, _) => 11.0,
            (AnnotationKind::Peak, _) => 10.5,
            (
                AnnotationKind::Health
                | AnnotationKind::Education
                | AnnotationKind::Landmark
                | AnnotationKind::Transit,
                _,
            ) => 10.0,
        };
        let typeface = if annotation.kind == AnnotationKind::Country || annotation.capital {
            &typefaces.bold
        } else {
            &typefaces.regular
        };
        let mut font = Font::new(typeface.clone(), logical_size * DEVICE_PIXEL_RATIO as f32);
        font.set_subpixel(true);
        let (width, _) = font.measure_str(&text, None);
        let (_, metrics) = font.metrics();
        let baseline = annotation.y - (metrics.ascent + metrics.descent) / 2.0;
        let marker = annotation.kind.marker();
        let marker_radius = marker.map_or(0.0, marker_radius);
        let marker_gap = marker.map_or(0.0, |_| 3.0 * DEVICE_PIXEL_RATIO as f32);
        let x = if marker.is_some() {
            annotation.x + marker_radius + marker_gap
        } else {
            annotation.x - width / 2.0
        };
        let padding = 3.5 * DEVICE_PIXEL_RATIO as f32;
        let bounds = LabelBounds {
            left: if marker.is_some() {
                annotation.x - marker_radius - padding
            } else {
                x - padding
            },
            top: baseline + metrics.ascent - padding,
            right: x + width + padding,
            bottom: baseline + metrics.descent + padding,
        };
        if !bounds.inside_frame()
            || occupied
                .iter()
                .copied()
                .any(|other| bounds.intersects(other))
        {
            continue;
        }

        if let Some(marker) = marker {
            draw_marker(
                canvas,
                annotation.x,
                annotation.y,
                marker,
                annotation.kind,
                palette,
            );
        }

        let mut halo = Paint::default();
        halo.set_anti_alias(true);
        halo.set_color(color(palette.label_halo));
        halo.set_style(PaintStyle::Stroke);
        halo.set_stroke_width(
            if annotation.kind.marker().is_some() {
                1.8
            } else {
                2.2
            } * DEVICE_PIXEL_RATIO as f32,
        );
        halo.set_stroke_join(LineJoin::Round);
        canvas.draw_str(&text, (x, baseline), &font, &halo);

        let mut fill = Paint::default();
        fill.set_anti_alias(true);
        fill.set_color(color(match annotation.kind {
            AnnotationKind::Country => palette.label_country,
            AnnotationKind::City => palette.label_city,
            AnnotationKind::Town
            | AnnotationKind::Village
            | AnnotationKind::Airport
            | AnnotationKind::Peak
            | AnnotationKind::Health
            | AnnotationKind::Education
            | AnnotationKind::Landmark
            | AnnotationKind::Transit => palette.label_secondary,
        }));
        canvas.draw_str(&text, (x, baseline), &font, &fill);
        occupied.push(bounds);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_annotation(
    annotations: &mut Vec<Annotation>,
    seen: &mut HashSet<(AnnotationKind, String)>,
    feature: MvtFeatureRef<'_>,
    kind: AnnotationKind,
    rank: f64,
    capital: bool,
    tile: &RenderTile,
    extent: u32,
    transform: &RenderTransform,
) -> MapResult<()> {
    let Some(name) = localized_name(feature) else {
        return Ok(());
    };
    if name.is_empty() {
        return Ok(());
    }
    let geometry = feature
        .geometry()
        .map_err(|error| MapError::new(format!("invalid vector geometry: {error}")))?;
    let Some(coordinate) = label_coordinate(&geometry) else {
        return Ok(());
    };
    let (x, y) = transform.point(&tile.placement, extent, coordinate);
    if x < -160.0 || x > FRAME_WIDTH as f32 + 160.0 || y < -80.0 || y > FRAME_HEIGHT as f32 + 80.0 {
        return Ok(());
    }
    if !seen.insert((kind, name.to_owned())) {
        return Ok(());
    }
    annotations.push(Annotation {
        name: name.to_owned(),
        kind,
        rank,
        capital,
        x,
        y,
    });
    Ok(())
}

fn localized_name(feature: MvtFeatureRef<'_>) -> Option<&str> {
    string_property(feature, "name:en")
        .or_else(|| string_property(feature, "name_en"))
        .or_else(|| string_property(feature, "name"))
}

fn place_kind(feature: MvtFeatureRef<'_>, zoom: f64) -> Option<AnnotationKind> {
    match string_property(feature, "class")? {
        "country" if zoom <= 7.5 => Some(AnnotationKind::Country),
        "city" if zoom >= 4.0 => Some(AnnotationKind::City),
        "town" if zoom >= 8.0 => Some(AnnotationKind::Town),
        "village" | "hamlet" if zoom >= 11.5 => Some(AnnotationKind::Village),
        _ => None,
    }
}

fn place_rank_visible(
    kind: AnnotationKind,
    rank: f64,
    capital: bool,
    zoom: f64,
    density: AnnotationDensity,
) -> bool {
    let rank_scale = density.rank_scale();
    match kind {
        AnnotationKind::Country => rank <= 6.0 * rank_scale,
        AnnotationKind::City if capital => true,
        AnnotationKind::City => {
            let maximum = if zoom < 6.0 {
                3.0
            } else if zoom < 8.0 {
                6.0
            } else if zoom < 10.0 {
                10.0
            } else if zoom < 12.0 {
                14.0
            } else {
                24.0
            };
            rank <= maximum * rank_scale
        }
        AnnotationKind::Town => {
            rank <= if zoom < 10.0 {
                5.0
            } else if zoom < 12.0 {
                10.0
            } else {
                20.0
            } * rank_scale
        }
        AnnotationKind::Village => rank <= if zoom < 13.0 { 10.0 } else { 20.0 } * rank_scale,
        AnnotationKind::Airport
        | AnnotationKind::Peak
        | AnnotationKind::Health
        | AnnotationKind::Education
        | AnnotationKind::Landmark
        | AnnotationKind::Transit => false,
    }
}

fn aerodrome_rank(feature: MvtFeatureRef<'_>, zoom: f64) -> Option<f64> {
    match string_property(feature, "class")? {
        "international" => Some(0.0),
        "public" | "regional" => Some(1.0),
        "military" if zoom >= 10.0 => Some(3.0),
        "private" | "other" if zoom >= 12.0 => Some(5.0),
        _ => None,
    }
}

fn peak_rank_visible(rank: f64, zoom: f64, density: AnnotationDensity) -> bool {
    rank <= if zoom < 11.0 {
        2.0
    } else if zoom < 13.0 {
        5.0
    } else {
        12.0
    } * density.rank_scale()
}

fn poi_kind(feature: MvtFeatureRef<'_>) -> Option<AnnotationKind> {
    let class = string_property(feature, "class").unwrap_or_default();
    let subclass = string_property(feature, "subclass").unwrap_or_default();
    match (class, subclass) {
        ("hospital" | "clinic", _) | (_, "hospital" | "clinic") => Some(AnnotationKind::Health),
        ("college" | "university" | "school", _) | (_, "college" | "university" | "school") => {
            Some(AnnotationKind::Education)
        }
        (
            "attraction" | "museum" | "art_gallery" | "castle" | "zoo" | "stadium" | "park"
            | "library" | "town_hall",
            _,
        )
        | (
            _,
            "attraction" | "museum" | "art_gallery" | "castle" | "zoo" | "stadium" | "park"
            | "library" | "town_hall",
        ) => Some(AnnotationKind::Landmark),
        ("railway" | "harbor", _) | (_, "railway" | "harbor") => Some(AnnotationKind::Transit),
        _ => None,
    }
}

fn poi_rank_visible(rank: f64, zoom: f64, density: AnnotationDensity) -> bool {
    rank <= if zoom < 13.0 {
        2.0
    } else if zoom < 14.0 {
        5.0
    } else {
        12.0
    } * density.rank_scale()
}

fn marker_radius(marker: MarkerKind) -> f32 {
    let logical = match marker {
        MarkerKind::Circle | MarkerKind::Square => 2.75,
        MarkerKind::Diamond | MarkerKind::Triangle | MarkerKind::Health => 3.5,
    };
    logical * DEVICE_PIXEL_RATIO as f32
}

fn draw_marker(
    canvas: &Canvas,
    x: f32,
    y: f32,
    marker: MarkerKind,
    kind: AnnotationKind,
    palette: Palette,
) {
    let radius = marker_radius(marker);
    let marker_color = match kind {
        AnnotationKind::Airport | AnnotationKind::Transit => palette.marker_transport,
        AnnotationKind::Health => palette.marker_health,
        _ => palette.marker_neutral,
    };

    let mut halo = Paint::default();
    halo.set_anti_alias(true);
    halo.set_color(color(palette.label_halo));
    halo.set_style(PaintStyle::Stroke);
    halo.set_stroke_width(1.7 * DEVICE_PIXEL_RATIO as f32);
    halo.set_stroke_join(LineJoin::Round);

    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    fill.set_color(color(marker_color));
    fill.set_style(PaintStyle::Fill);

    match marker {
        MarkerKind::Circle => {
            canvas.draw_circle((x, y), radius, &halo);
            canvas.draw_circle((x, y), radius, &fill);
        }
        MarkerKind::Square => {
            let rect = Rect::from_xywh(x - radius, y - radius, radius * 2.0, radius * 2.0);
            canvas.draw_rect(rect, &halo);
            canvas.draw_rect(rect, &fill);
        }
        MarkerKind::Diamond | MarkerKind::Triangle => {
            let mut builder = PathBuilder::new();
            if matches!(marker, MarkerKind::Diamond) {
                builder.move_to((x, y - radius));
                builder.line_to((x + radius, y));
                builder.line_to((x, y + radius));
                builder.line_to((x - radius, y));
            } else {
                builder.move_to((x, y - radius));
                builder.line_to((x + radius, y + radius));
                builder.line_to((x - radius, y + radius));
            }
            builder.close();
            let path = builder.detach();
            canvas.draw_path(&path, &halo);
            canvas.draw_path(&path, &fill);
        }
        MarkerKind::Health => {
            canvas.draw_circle((x, y), radius, &halo);
            canvas.draw_circle((x, y), radius, &fill);
            let mut cross = Paint::default();
            cross.set_anti_alias(true);
            cross.set_color(color(palette.label_halo));
            cross.set_style(PaintStyle::Stroke);
            cross.set_stroke_width(1.25 * DEVICE_PIXEL_RATIO as f32);
            cross.set_stroke_cap(LineCap::Round);
            let arm = radius * 0.48;
            canvas.draw_line((x - arm, y), (x + arm, y), &cross);
            canvas.draw_line((x, y - arm), (x, y + arm), &cross);
        }
    }
}

fn label_coordinate(geometry: &MvtGeometry) -> Option<fast_mvt::MvtCoord> {
    match geometry {
        MvtGeometry::Point(point) => Some(point.0),
        MvtGeometry::MultiPoint(points) => points.0.first().map(|point| point.0),
        _ => None,
    }
}

fn append_line(
    builder: &mut PathBuilder,
    line: &fast_mvt::MvtLineString,
    placement: &TilePlacement,
    extent: u32,
    transform: &RenderTransform,
    close: bool,
) {
    let Some((first, rest)) = line.0.split_first() else {
        return;
    };
    let (x, y) = transform.point(placement, extent, *first);
    builder.move_to((x, y));
    for coordinate in rest {
        let (x, y) = transform.point(placement, extent, *coordinate);
        builder.line_to((x, y));
    }
    if close {
        builder.close();
    }
}

fn road_style(
    feature: MvtFeatureRef<'_>,
    zoom: f64,
    palette: Palette,
    emphasis: RoadEmphasis,
    casing: bool,
) -> Option<LineStyle> {
    let class = string_property(feature, "class")?;
    let zoom_scale =
        2_f32.powf((zoom as f32 - 12.0) * 0.16).clamp(0.55, 3.2) * emphasis.width_scale();
    let (inner_color, logical_width, has_casing) = match class {
        "motorway" => (palette.motorway, 2.6, true),
        "trunk" | "primary" => (palette.road_major, 2.25, true),
        "secondary" => (palette.road_major, 1.9, true),
        "tertiary" => (palette.road_major, 1.6, true),
        "minor" => (palette.road_minor, 1.15, true),
        "service" | "track" => (palette.road_minor, 0.9, false),
        "path" | "pedestrian" if zoom >= 13.0 => (palette.road_minor, 0.7, false),
        "rail" | "transit" => (palette.rail, 0.85, false),
        _ => return None,
    };

    if casing {
        has_casing.then_some(
            LineStyle::solid(
                palette.road_casing,
                (logical_width * zoom_scale + 0.55 * emphasis.width_scale())
                    * DEVICE_PIXEL_RATIO as f32,
            )
            .with_line_cap(LineCap::Butt),
        )
    } else {
        Some(
            LineStyle::solid(
                inner_color,
                logical_width * zoom_scale * DEVICE_PIXEL_RATIO as f32,
            )
            .with_line_cap(LineCap::Butt),
        )
    }
}

#[derive(Clone, Copy)]
struct LineStyle {
    color: [u8; 4],
    width: f32,
    dash: Option<[f32; 2]>,
    line_cap: LineCap,
}

impl LineStyle {
    fn solid(color: [u8; 4], width: f32) -> Self {
        Self {
            color,
            width,
            dash: None,
            line_cap: LineCap::Round,
        }
    }

    fn dashed(color: [u8; 4], width: f32, dash: [f32; 2]) -> Self {
        Self {
            color,
            width,
            dash: Some(dash),
            line_cap: LineCap::Round,
        }
    }

    fn with_line_cap(mut self, line_cap: LineCap) -> Self {
        self.line_cap = line_cap;
        self
    }
}

fn string_property<'a>(feature: MvtFeatureRef<'a>, name: &str) -> Option<&'a str> {
    feature.properties().find_map(|property| match property {
        Ok((key, MvtValueRef::String(value))) if key == name => Some(value),
        _ => None,
    })
}

fn number_property(feature: MvtFeatureRef<'_>, name: &str) -> Option<f64> {
    feature.properties().find_map(|property| match property {
        Ok((key, MvtValueRef::Float(value))) if key == name => Some(f64::from(value)),
        Ok((key, MvtValueRef::Double(value))) if key == name => Some(value),
        Ok((key, MvtValueRef::Int(value) | MvtValueRef::SInt(value))) if key == name => {
            Some(value as f64)
        }
        Ok((key, MvtValueRef::UInt(value))) if key == name => Some(value as f64),
        _ => None,
    })
}

fn color([red, green, blue, alpha]: [u8; 4]) -> Color {
    Color::from_argb(alpha, red, green, blue)
}

fn frame_image_info() -> ImageInfo {
    ImageInfo::new(
        (FRAME_WIDTH as i32, FRAME_HEIGHT as i32),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    )
}
