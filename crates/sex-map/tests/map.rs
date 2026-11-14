// SPDX-License-Identifier: GPL-3.0-only
//! The map model on the host, against the fixtures scripts/make_map_fixture.py wrote
//! (its encoder is independent of this decoder).
use sex_map::mvt::{self, GeomType, Value};
use sex_map::render::{self, Text};
use sex_map::{geo, http, offline, route, search, Servers, TileId, View};

const TILE: &[u8] = include_bytes!("fixtures/tile-14-8800-5373.mvt");
const TILE_GZ: &[u8] = include_bytes!("fixtures/tile-14-8800-5373.mvt.gz");
const FOUNTAIN: &[u8] = include_bytes!("fixtures/search-fountain.json");
const STATION: &[u8] = include_bytes!("fixtures/search-station.json");
const ROUTE: &[u8] = include_bytes!("fixtures/route.json");

fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> u32 {
    let p = pm.pixel(x, y).unwrap();
    (p.red() as u32) << 16 | (p.green() as u32) << 8 | p.blue() as u32
}

#[test]
fn decodes_the_fixture_tile() {
    let t = mvt::decode(TILE).unwrap();
    let names: Vec<&str> = t.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["water", "landcover", "park", "building", "transportation", "transportation_name", "water_name", "place"]);
    assert_eq!(t.features(), 13);
    let b = t.layer("building").unwrap();
    assert_eq!(b.extent, 4096);
    // The courtyard building: an exterior ring and a hole, closed, opposite windings.
    let court = &b.features[1];
    assert_eq!((court.kind, court.parts.len()), (GeomType::Polygon, 2));
    assert_eq!(court.parts[0], [(3200, 2400), (3900, 2400), (3900, 3100), (3200, 3100), (3200, 2400)]);
    assert!(mvt::ring_area(&court.parts[0]) > 0 && mvt::ring_area(&court.parts[1]) < 0);
    assert_eq!(court.get("render_height"), Some(&Value::Int(12)));
    let roads = t.layer("transportation").unwrap();
    assert_eq!(roads.features[2].parts.len(), 2, "a multi-line");
    assert_eq!(roads.features[0].str("class"), Some("primary"));
    let place = &t.layer("place").unwrap().features[0];
    assert_eq!((place.kind, place.str("name"), place.parts.clone()), (GeomType::Point, Some("Fountain Square"), vec![vec![(2048, 1800)]]));
    // The gzipped copy, as servers send it, is the same tile.
    assert_eq!(mvt::decode(&sex_map::inflate(TILE_GZ).unwrap()).unwrap(), t);
    assert_eq!(sex_map::inflate(TILE).unwrap(), TILE);
}

#[test]
fn refuses_broken_tiles() {
    for n in 1..TILE.len() {
        // Every truncation either fails cleanly or decodes fewer layers; none panics.
        if let Ok(t) = mvt::decode(&TILE[..n]) { assert!(t.layers.len() <= 8); }
    }
    assert!(mvt::geometry(&[9, 2], GeomType::Point).is_err(), "MoveTo without its parameters");
    assert!(mvt::geometry(&[15], GeomType::Polygon).is_err(), "ClosePath before any point");
    assert!(mvt::decode(&[0x1A, 0x05, 0x0A, 0x10]).is_err());
}

#[test]
fn renders_water_roads_buildings_and_labels() {
    let t = mvt::decode(TILE).unwrap();
    let mut pm = render::blank(512, 512).unwrap();
    let labels = render::draw_vector(&mut pm, &t, 0.0, 0.0, 512.0);
    // The tile is 4096 units over 512 pixels: 8 units a pixel.
    assert_eq!(px(&pm, 600 / 8, 1000 / 8), render::WATER);
    assert_eq!(px(&pm, 1800 / 8, 800 / 8), render::PARK);
    assert_eq!(px(&pm, 1600 / 8, 2048 / 8), render::MAJOR_ROAD, "primary road");
    assert_eq!(px(&pm, 3000 / 8, 1000 / 8), render::MAJOR_ROAD, "secondary road");
    assert_eq!(px(&pm, 2700 / 8, 2600 / 8), render::BUILDING);
    assert_eq!(px(&pm, 3550 / 8, 2750 / 8), render::LAND, "the courtyard is not filled");
    assert_eq!(px(&pm, 2000 / 8, 3500 / 8), render::MINOR_ROAD);
    assert_eq!(px(&pm, 2000 / 8, 3200 / 8), render::LAND);
    // The road edges are cased.
    assert_ne!(px(&pm, 1600 / 8, 2048 / 8 - 3), render::LAND);
    let mut names: Vec<&str> = labels.iter().map(|l| l.text.as_str()).collect();
    names.sort();
    assert_eq!(names, ["Blue River", "Fountain Square", "Main Street"]);
    let mut text = Text::new(7, 14).unwrap();
    let drawn = text.place(&mut pm, labels);
    assert_eq!(drawn, 3);
    let dark = pm.pixels().iter().filter(|p| p.red() < 0x80 && p.green() < 0x80 && p.blue() < 0x80).count();
    assert!(dark > 30, "label ink: {dark}");
    let mut out = vec![0u32; 512 * 512];
    render::to_window(&pm, &mut out);
    assert_eq!(out[(2500 / 8) * 512 + 300 / 8], render::WATER);
}

#[test]
fn raster_fallback_is_stretched() {
    let mut pm = render::blank(300, 300).unwrap();
    let tile: Vec<u32> = (0..256 * 256).map(|i| if (i % 256) < 128 { 0xFF11_2233 } else { 0xFF44_5566 }).collect();
    render::draw_raster(&mut pm, &tile, 256, 256, 10.0, 10.0, 200.0);
    assert_eq!(px(&pm, 5, 5), render::LAND);
    assert_eq!(px(&pm, 50, 100), 0x0011_2233);
    assert_eq!(px(&pm, 150, 100), 0x0044_5566);
    assert_eq!(px(&pm, 250, 100), render::LAND);
}

#[test]
fn mercator_and_view() {
    let (lat, lon) = (52.5163, 13.3777);
    assert_eq!(geo::tile_at(lat, lon, 14), TileId { z: 14, x: 8800, y: 5373 });
    let (wx, wy) = geo::world(lat, lon);
    let back = geo::lat_lon(wx, wy);
    assert!((back.0 - lat).abs() < 1e-9 && (back.1 - lon).abs() < 1e-9);
    let (n, w, s, e) = geo::tile_bounds(TileId { z: 14, x: 8800, y: 5373 });
    assert!(n > lat && s < lat && w < lon && e > lon);
    // A kilometre is about a kilometre.
    let d = geo::distance((52.0, 13.0), (52.0 + 1.0 / 111.2, 13.0));
    assert!((d - 1000.0).abs() < 5.0, "{d}");

    let mut v = View::new(lat, lon, 15.0, 800, 600);
    let (x, y) = v.to_screen(lat, lon);
    assert!((x - 400.0).abs() < 1e-6 && (y - 300.0).abs() < 1e-6);
    // Zoom 15 over zoom-14 tiles: each is 512 pixels; 800x600 needs 2 or 3 across.
    let tiles = v.tiles(v.tile_zoom(14));
    assert!(tiles.iter().all(|t| t.0.z == 14 && (t.3 - 512.0).abs() < 1e-9));
    assert_eq!(tiles[0].0, TileId { z: 14, x: 8800, y: 5373 }, "the centre tile comes first");
    assert!((4..=9).contains(&tiles.len()));
    // Dragging moves the place with the pointer.
    v.pan(100.0, -50.0);
    let (x, y) = v.to_screen(lat, lon);
    assert!((x - 500.0).abs() < 1e-6 && (y - 250.0).abs() < 1e-6);
    // Zooming about a point keeps that point still.
    let under = v.to_lat_lon(123.0, 456.0);
    v.zoom_at(1.5, 123.0, 456.0);
    let (x, y) = v.to_screen(under.0, under.1);
    assert!((x - 123.0).abs() < 1e-6 && (y - 456.0).abs() < 1e-6);
    assert_eq!(v.zoom, 16.5);
    assert_eq!(v.tile_zoom(14), 14);
    v.zoom_at(-20.0, 0.0, 0.0);
    assert_eq!(v.zoom, sex_map::MIN_ZOOM);
}

#[test]
fn search_and_route_answers() {
    assert_eq!(search::url("http://10.0.2.2:8080/", "Fountain Sq. & café", 5), "http://10.0.2.2:8080/search?q=Fountain+Sq.+%26+caf%C3%A9&format=jsonv2&limit=5");
    let f = search::parse(FOUNTAIN).unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!((f[0].short(), f[0].kind.as_str()), ("Fountain Square", "amenity/fountain"));
    let s = search::parse(STATION).unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].short(), "Old Mill Station");
    assert_eq!(geo::tile_at(f[0].lat, f[0].lon, 14), TileId { z: 14, x: 8800, y: 5373 });
    assert!(search::parse(b"{}").is_err());
    assert!(search::parse(br#"[{"lat":"x","lon":"1"},{"lat":"1.5","lon":2}]"#).unwrap().len() == 1);

    let u = route::url("http://h", "driving", (f[0].lat, f[0].lon), (s[0].lat, s[0].lon));
    assert!(u.starts_with("http://h/route/v1/driving/13.370361,52.517030;13.375468,52.513113?"), "{u}");
    let r = route::parse(ROUTE).unwrap();
    assert_eq!(r.points.len(), 4);
    assert!((r.points[0].0 - f[0].lat).abs() < 1e-5 && (r.points[3].1 - s[0].lon).abs() < 1e-5);
    assert_eq!((r.distance, r.duration), (812.4, 95.1));
    assert_eq!(route::parse(br#"{"code":"NoRoute","message":"Impossible route"}"#), Err(route::Error::NoRoute("Impossible route".into())));
    // GeoJSON geometry works too.
    let g = route::parse(br#"{"code":"Ok","routes":[{"distance":1,"duration":2,"geometry":{"type":"LineString","coordinates":[[13.1,52.1],[13.2,52.2]]}}]}"#).unwrap();
    assert_eq!(g.points, [(52.1, 13.1), (52.2, 13.2)]);
}

#[test]
fn polyline_known_answer() {
    // The format's documented example.
    let p = route::decode_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@", 5).unwrap();
    assert_eq!(p, [(38.5, -120.2), (40.7, -120.95), (43.252, -126.453)]);
    assert_eq!(route::encode_polyline(&p, 5), "_p~iF~ps|U_ulLnnqC_mqNvxq`@");
    assert!(route::decode_polyline("_p~iF~ps|U_", 5).is_err());
}

#[test]
fn route_line_is_drawn() {
    let r = route::parse(ROUTE).unwrap();
    let v = View::new(r.points[1].0, r.points[1].1, 16.0, 400, 400);
    let mut pm = render::blank(400, 400).unwrap();
    render::draw_route(&mut pm, &v, &r.points);
    assert!(render::count(&pm, render::ROUTE) > 1000);
    render::draw_marker(&mut pm, &v, r.points[0].0, r.points[0].1, render::MARKER);
    assert!(render::count(&pm, render::MARKER) > 50);
}

#[test]
fn http_responses() {
    let u = http::parse_url("http://10.0.2.2:8123/tiles/14/1/2.mvt").unwrap();
    assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("10.0.2.2", 8123, "/tiles/14/1/2.mvt"));
    assert_eq!(http::parse_url("http://example.org").unwrap().path, "/");
    assert_eq!(http::parse_url("https://example.org/"), Err(http::Error::NotHttp));
    assert!(http::request(&u).starts_with("GET /tiles/14/1/2.mvt HTTP/1.1\r\nHost: 10.0.2.2:8123\r\n"));
    let r = http::parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Encoding: gzip\r\n\r\nhelloEXTRA").unwrap();
    assert_eq!((r.status, r.body.as_slice(), r.header("content-encoding")), (200, &b"hello"[..], Some("gzip")));
    let r = http::parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n6;x=y\r\npedia \r\n0\r\n\r\n").unwrap();
    assert_eq!(r.body, b"Wikipedia ");
    assert_eq!(http::parse_response(b"HTTP/1.0 404 Not Found\r\n\r\n").unwrap().status, 404);
    assert_eq!(http::parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nshort"), Err(http::Error::Truncated));
}

#[test]
fn servers_and_offline_regions() {
    let s = Servers::at("http://10.0.2.2:9000/");
    assert_eq!(Servers::tile_url(&s.vector, TileId { z: 14, x: 8800, y: 5373 }), "http://10.0.2.2:9000/tiles/14/8800/5373.mvt");
    assert_eq!(Servers::parse(&s.serialize()), s);
    let custom = Servers::parse("base=http://a\nraster=http://b/{z}/{x}/{y}.png\nmax_zoom=16\n");
    assert_eq!((custom.search.as_str(), custom.raster.as_str(), custom.max_zoom), ("http://a", "http://b/{z}/{x}/{y}.png", 16));

    let r = offline::Region { name: "Mill Quarter".into(), north: 52.52, west: 13.36, south: 52.51, east: 13.38, min_zoom: 13, max_zoom: 15 };
    let tiles = r.tiles().unwrap();
    assert!(tiles.contains(&TileId { z: 14, x: 8800, y: 5373 }));
    assert!(tiles.windows(2).all(|w| w[0].z <= w[1].z));
    assert!(r.contains(52.515, 13.37) && !r.contains(52.6, 13.37));
    let back = offline::parse(&offline::serialize(core::slice::from_ref(&r)));
    assert_eq!(back.len(), 1);
    assert_eq!((back[0].name.as_str(), back[0].max_zoom), ("Mill Quarter", 15));
    assert_eq!(offline::tile_path(TileId { z: 14, x: 8800, y: 5373 }, offline::Kind::Vector), "offline/14/8800/5373.mvt");
    let huge = offline::Region { name: "World".into(), north: 80.0, west: -170.0, south: -80.0, east: 170.0, min_zoom: 0, max_zoom: 12 };
    assert!(huge.tiles().is_none());
}
