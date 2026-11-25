// SPDX-License-Identifier: GPL-3.0-only
//! Small, interoperable ODF chart subset. Reject anything we cannot round trip.
use super::{attr, escape, length_px};
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::{Anchor, Chart, ChartKind, LegendPos, Series, Workbook};
use quick_xml::events::Event;
use std::{fmt::Write as _, io::Read};

#[derive(Clone, Debug, PartialEq)]
pub struct FoundChart { pub sheet: usize, pub chart: Chart }

pub struct ChartParts { pub frames: Vec<String>, pub manifest: String, pub objects: Vec<(String, String)> }

fn supported(chart: &Chart) -> Result<&'static str, String> {
    let kind = match chart.kind {
        ChartKind::ColumnClustered => "chart:bar",
        ChartKind::Line => "chart:line",
        ChartKind::Pie => "chart:circle",
        _ => return Err("ODS chart export supports plain column, line and pie charts; save as XLSX to keep this chart".into()),
    };
    if chart.style != 1 || chart.data_labels || chart.data_table.is_some() || chart.x_title.is_some()
        || chart.y_title.is_some() || chart.pivot.is_some() || chart.hole_size != 75
        || chart.first_slice_angle != 0 || chart.by_rows {
        return Err("ODS cannot preserve this chart's design settings; save as XLSX".into());
    }
    if chart.series.is_empty() || chart.series.iter().any(|s| s.color.is_some() || s.secondary
        || s.kind.is_some() || s.smooth || !s.trendlines.is_empty() || s.bubble_sizes.is_some()) {
        return Err("ODS cannot preserve this chart's series settings; save as XLSX".into());
    }
    if chart.kind == ChartKind::Pie && chart.series.len() != 1 {
        return Err("ODS cannot preserve a pie chart with multiple series; save as XLSX".into());
    }
    Ok(kind)
}

fn ref_addr(reference: &str) -> Result<String, String> {
    let (sheet, cells) = reference.rsplit_once('!').ok_or("ODS chart data must use a sheet cell range")?;
    let sheet = sheet.trim_matches('\'').replace("''", "'");
    let (first, last) = cells.split_once(':').unwrap_or((cells, cells));
    let first = CellRef::parse(first).ok_or("ODS chart data has an invalid first cell")?;
    let last = CellRef::parse(last).ok_or("ODS chart data has an invalid last cell")?;
    let name = if sheet.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { sheet.to_string() }
        else { format!("'{}'", sheet.replace('\'', "''")) };
    Ok(format!("${name}.{}:${name}.{}", first.a1(), last.a1()))
}

pub fn write(wb: &Workbook) -> Result<ChartParts, String> {
    let mut parts = ChartParts { frames: vec![String::new(); wb.sheets.len()], manifest: String::new(), objects: Vec::new() };
    let mut number = 0;
    for (sheet_index, sheet) in wb.sheets.iter().enumerate() {
        if sheet.charts.is_empty() { continue; }
        parts.frames[sheet_index].push_str("<table:shapes>");
        for chart in &sheet.charts {
            let kind = supported(chart)?;
            number += 1;
            let object = format!("Object {number}");
            let name = &sheet.name;
            let anchor = chart.anchor;
            let x = (anchor.dx + (0..anchor.cell.col).map(|c| sheet.col_width(c) * 0.75).sum::<f32>()) / 72.0;
            let y = (anchor.dy + (0..anchor.cell.row).map(|r| sheet.row_height(r) * 0.75).sum::<f32>()) / 72.0;
            let width = anchor.width / 72.0;
            let height = anchor.height / 72.0;
            if ![x, y, width, height].into_iter().all(f32::is_finite) || width <= 0.0 || height <= 0.0 {
                return Err("ODS chart has an invalid position or size".into());
            }
            let cell = anchor.cell.a1();
            let _ = write!(parts.frames[sheet_index], "<draw:frame draw:name=\"Spreadeagle Chart {number}\" table:anchor-type=\"cell\" table:cell-address=\"{}\" table:end-cell-address=\"{}\" table:end-x=\"0in\" table:end-y=\"0in\" svg:x=\"{x:.5}in\" svg:y=\"{y:.5}in\" svg:width=\"{width:.5}in\" svg:height=\"{height:.5}in\"><draw:object xlink:href=\"./{object}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/></draw:frame>", escape(&format!("${name}.{cell}")), escape(&format!("${name}.{cell}")));
            let mut plot = String::new();
            let mut categories = None;
            for series in &chart.series {
                let values = ref_addr(&series.values)?;
                if let Some(cat) = &series.categories {
                    let cat = ref_addr(cat)?;
                    if categories.as_ref().is_some_and(|existing| existing != &cat) {
                        return Err("ODS cannot preserve differing category ranges; save as XLSX".into());
                    }
                    categories = Some(cat);
                }
                let label = match &series.name {
                    Some(n) => format!(" chart:label-cell-address=\"{}\"", escape(&ref_addr(n.trim_start_matches('='))?)),
                    None => String::new(),
                };
                let _ = write!(plot, "<chart:series chart:class=\"{kind}\" chart:values-cell-range-address=\"{}\"{label}/>", escape(&values));
            }
            let category_axis = categories.map(|cat| format!("<chart:axis chart:dimension=\"x\" chart:name=\"primary-x\"><chart:categories table:cell-range-address=\"{}\"/></chart:axis>", escape(&cat)))
                .unwrap_or_else(|| "<chart:axis chart:dimension=\"x\" chart:name=\"primary-x\"/>".into());
            let grid = if chart.gridlines { "<chart:grid chart:class=\"major\"/>" } else { "" };
            plot.insert_str(0, &format!("{category_axis}<chart:axis chart:dimension=\"y\" chart:name=\"primary-y\">{grid}</chart:axis>"));
            let title = chart.title.as_ref().map(|t| format!("<chart:title><text:p>{}</text:p></chart:title>", escape(t))).unwrap_or_default();
            let legend = match chart.legend { LegendPos::None => "", LegendPos::Bottom => "bottom", LegendPos::Top => "top", LegendPos::Left => "start", LegendPos::Right => "end" };
            let legend = if legend.is_empty() { String::new() } else { format!("<chart:legend chart:legend-position=\"{legend}\"/>") };
            let object_xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:chart=\"urn:oasis:names:tc:opendocument:xmlns:chart:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" office:version=\"1.3\"><office:body><office:chart><chart:chart chart:class=\"{kind}\" svg:width=\"{width:.5}in\" svg:height=\"{height:.5}in\">{title}{legend}<chart:plot-area>{plot}</chart:plot-area></chart:chart></office:chart></office:body></office:document-content>");
            parts.objects.push((format!("{object}/content.xml"), object_xml));
            let _ = write!(parts.manifest, "<manifest:file-entry manifest:full-path=\"{object}/\" manifest:media-type=\"application/vnd.oasis.opendocument.chart\"/><manifest:file-entry manifest:full-path=\"{object}/content.xml\" manifest:media-type=\"text/xml\"/>");
        }
        parts.frames[sheet_index].push_str("</table:shapes>");
    }
    Ok(parts)
}

pub fn read<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, content: &str, names: &[String]) -> Result<(Vec<FoundChart>, Vec<String>), String> {
    let mut reader = quick_xml::Reader::from_str(content);
    let mut sheet = None;
    let mut frame = None;
    let mut references = Vec::new();
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                b"table" => sheet = sheet.map_or(Some(0), |s: usize| Some(s + 1)),
                b"frame" => frame = Some((sheet, attr(&e, b"cell-address"), attr(&e, b"x"), attr(&e, b"y"), attr(&e, b"width"), attr(&e, b"height"))),
                b"object" => if let (Some((Some(s), _, x, y, width, height)), Some(href)) = (frame.clone(), attr(&e, b"href")) {
                    references.push((s, x, y, width, height, href));
                },
                _ => {},
            },
            Event::End(e) if e.local_name().as_ref() == b"frame" => frame = None,
            Event::Eof => break,
            _ => {},
        }
    }
    let mut charts = Vec::new();
    let mut warnings = Vec::new();
    for (sheet, x, y, width, height, href) in references {
        let object = href.trim_start_matches("./").trim_end_matches('/');
        let path = format!("{object}/content.xml");
        let mut xml = String::new();
        if zip.by_name(&path).ok().and_then(|mut file| file.read_to_string(&mut xml).ok()).is_none() {
            warnings.push(format!("ODS chart object {href} could not be opened; save as XLSX after rebuilding it"));
            continue;
        }
        // ODF stores plot settings in automatic styles, either in content.xml
        // or in the embedded chart's styles.xml. A clustered chart cannot
        // represent horizontal, stacked, percentage, or 3-D plot semantics.
        let mut styles = String::new();
        let _ = zip.by_name(&format!("{object}/styles.xml"))
            .ok().and_then(|mut file| file.read_to_string(&mut styles).ok());
        if unsupported_plot_semantics(&xml) || unsupported_plot_semantics(&styles) {
            warnings.push(format!("ODS chart object {href} has plot settings Spreadeagle cannot preserve; keep the original ODS file"));
            continue;
        }
        let inch = |v: Option<String>| v.as_deref().and_then(length_px).map(|n| n * 0.75).unwrap_or(0.0);
        // `svg:x/y` in table:shapes are absolute sheet coordinates. The Book
        // resolves them against its imported row and column geometry.
        let anchor = Anchor { cell: CellRef::new(0, 0), dx: inch(x), dy: inch(y), width: inch(width), height: inch(height), mode: Default::default() };
        match parse_object(&xml, names, anchor, charts.len() as u32 + 1) {
            Some(chart) => {
                if xml.contains("chart:style-name=") || xml.contains("style:style") {
                    warnings.push(format!("ODS chart object {href}: custom styling and axis settings may change on save; keep the original file until reviewed"));
                }
                charts.push(FoundChart { sheet, chart });
            },
            None => warnings.push(format!("ODS chart object {href} uses settings Spreadeagle cannot preserve; keep the original ODS file")),
        }
    }
    Ok((charts, warnings))
}

fn parse_object(xml: &str, names: &[String], anchor: Anchor, id: u32) -> Option<Chart> {
    // ODF uses the same `chart:bar` class for columns and horizontal bars.
    // The latter has a vertical category axis and needs a separate mapping.
    let mut reader = quick_xml::Reader::from_str(xml);
    let (mut kind, mut title, mut in_title, mut legend, mut series, mut categories) = (None, None, false, LegendPos::None, Vec::new(), None);
    loop {
        match reader.read_event().ok()? {
            Event::Start(ref e) | Event::Empty(ref e) => match e.local_name().as_ref() {
                b"chart" if e.name().as_ref() == b"chart:chart" => kind = match attr(e, b"class")?.as_str() { "chart:bar" => Some(ChartKind::ColumnClustered), "chart:line" => Some(ChartKind::Line), "chart:circle" => Some(ChartKind::Pie), _ => return None },
                b"title" => in_title = true,
                b"legend" => legend = match attr(e, b"legend-position").as_deref() { Some("bottom") => LegendPos::Bottom, Some("top") => LegendPos::Top, Some("start") => LegendPos::Left, Some("end") => LegendPos::Right, _ => return None },
                b"series" => {
                    let expected = match kind? { ChartKind::ColumnClustered => "chart:bar", ChartKind::Line => "chart:line", ChartKind::Pie => "chart:circle", _ => return None };
                    if attr(e, b"class").as_deref() != Some(expected) { return None; }
                    let values = from_addr(&attr(e, b"values-cell-range-address")?, names)?;
                    let name = match attr(e, b"label-cell-address") { Some(a) => Some(format!("={}", from_addr(&a, names)?)), None => None };
                    series.push(Series { name, categories: None, values, bubble_sizes: None, color: None, secondary: false, kind: None, smooth: false, quartiles: Default::default(), trendlines: Vec::new() });
                }
                b"categories" => categories = Some(from_addr(&attr(e, b"cell-range-address")?, names)?),
                _ => {},
            },
            Event::Text(t) if in_title => title = Some(quick_xml::escape::unescape(&String::from_utf8_lossy(t.as_ref())).ok()?.into_owned()),
            Event::End(e) if e.local_name().as_ref() == b"title" => in_title = false,
            Event::Eof => break,
            _ => {},
        }
    }
    if series.is_empty() || anchor.width <= 0.0 || anchor.height <= 0.0 || (kind == Some(ChartKind::Pie) && series.len() != 1) { return None; }
    for item in &mut series { item.categories = categories.clone(); }
    Some(Chart { id, kind: kind?, anchor, title, series, legend, data_labels: false, data_table: None,
        gridlines: xml.contains("<chart:grid"), style: 1, hole_size: 75, first_slice_angle: 0, x_title: None, y_title: None,
        source: None, by_rows: false, pivot: None })
}

fn unsupported_plot_semantics(xml: &str) -> bool {
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) => {
                if e.attributes().flatten().any(|a| {
                    (matches!(a.key.local_name().as_ref(), b"vertical" | b"stacked" | b"percentage" | b"three-dimensional")
                        && a.value.as_ref() == b"true")
                        || matches!(a.key.local_name().as_ref(), b"pie-offset" | b"starting-angle" | b"hole-size")
                }) { return true; }
            }
            Ok(Event::Eof) => return false,
            Err(_) => return true,
            _ => {},
        }
    }
}

fn from_addr(address: &str, names: &[String]) -> Option<String> {
    let (sheet, range) = super::odf_filter_range(address, names)?;
    let abs = |at: CellRef| { let a = at.a1(); let split = a.find(|c: char| c.is_ascii_digit()).unwrap_or(a.len()); format!("${}${}", &a[..split], &a[split..]) };
    let name = &names[sheet];
    let name = if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { name.clone() }
        else { format!("'{}'", name.replace('\'', "''")) };
    if range.start == range.end { Some(format!("{name}!{}", abs(range.start))) }
    else { Some(format!("{name}!{}:{}", abs(range.start), abs(range.end))) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn column_and_line_chart_objects_round_trip() {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        for (cell, input) in [("A1", "Month"), ("B1", "Sales"), ("A2", "Jan"), ("B2", "12"),
            ("A3", "Feb"), ("B3", "18"), ("A4", "Mar"), ("B4", "24")] {
            session.execute_headless("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        for (kind, at) in [("column", "D2"), ("line", "D20")] {
            session.execute_headless("insert.chart", json!({"range":"A1:B4", "type":kind, "at":at})).unwrap();
        }
        let original = &session.doc().unwrap().wb;
        assert_eq!(from_addr("$Sheet1.B2:$Sheet1.B4", &["Sheet1".into()]), Some("Sheet1!$B$2:$B$4".into()));
        let bytes = super::super::write(original).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut xml = String::new();
        zip.by_name("content.xml").unwrap().read_to_string(&mut xml).unwrap();
        let (found, warnings) = read(&mut zip, &xml, &["Sheet1".into()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?} object={}", { let mut s=String::new(); zip.by_name("Object 1/content.xml").unwrap().read_to_string(&mut s).unwrap(); s });
        assert_eq!(found.len(), 2);
        for (index, found) in found.iter().enumerate() {
            assert_eq!(found.chart.kind, original.sheets[0].charts[index].kind);
            assert_eq!(found.chart.series[0].values, original.sheets[0].charts[index].series[0].values);
            assert_eq!(found.chart.title, original.sheets[0].charts[index].title);
        }
        assert!(xml.contains("<table:shapes>"));
        assert!(zip.by_name("Object 1/content.xml").is_ok());
        let path = std::env::temp_dir().join(format!("spreadeagle-chart-{}.ods", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let (book, warnings) = crate::book::Book::open(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(warnings.iter().all(|w| !w.contains("could not") && !w.contains("cannot")), "{warnings:?}");
        assert_eq!(book.charts().len(), 2);
        assert_eq!(book.charts()[0].col, original.sheets[0].charts[0].anchor.cell.col);
        assert_eq!(book.charts()[1].row, original.sheets[0].charts[1].anchor.cell.row);
    }

    #[test]
    fn unsupported_chart_kind_refuses_ods_save() {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        for (cell, input) in [("A1", "Month"), ("B1", "Sales"), ("A2", "Jan"), ("B2", "12"),
            ("A3", "Feb"), ("B3", "18")] {
            session.execute_headless("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        session.execute_headless("insert.chart", json!({"range":"A1:B3", "type":"doughnut", "at":"D2"})).unwrap();
        assert!(super::super::write(&session.doc().unwrap().wb).unwrap_err().contains("save as XLSX"));
    }

    #[test]
    fn pie_chart_round_trips_with_categories_and_position() {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        for (cell, input) in [("A1", "Month"), ("B1", "Sales"), ("A2", "Jan"), ("B2", "12"),
            ("A3", "Feb"), ("B3", "18")] {
            session.execute_headless("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        session.execute_headless("insert.chart", json!({"range":"A1:B3", "type":"pie", "at":"D2"})).unwrap();
        let original = &session.doc().unwrap().wb.sheets[0].charts[0];
        let bytes = super::super::write(&session.doc().unwrap().wb).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut content = String::new();
        zip.by_name("content.xml").unwrap().read_to_string(&mut content).unwrap();
        let mut object = String::new();
        zip.by_name("Object 1/content.xml").unwrap().read_to_string(&mut object).unwrap();
        assert!(object.contains("chart:class=\"chart:circle\""));
        assert!(object.contains("<chart:categories"));
        let (found, warnings) = read(&mut zip, &content, &["Sheet1".into()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].chart.kind, ChartKind::Pie);
        assert_eq!(found[0].chart.series[0].categories, original.series[0].categories);
        assert_eq!(found[0].chart.series[0].values, original.series[0].values);
        assert!(found[0].chart.anchor.dx > 0.0);
    }

    #[test]
    fn libreoffice_resaved_pie_fixture_opens_and_round_trips() {
        let path = std::env::temp_dir().join(format!("spreadeagle-lo-pie-{}.ods", std::process::id()));
        std::fs::write(&path, include_bytes!("../../assets/lo-pie.ods")).unwrap();
        let (mut book, warnings) = crate::book::Book::open(path.to_str().unwrap()).unwrap();
        assert!(warnings.iter().all(|w| !w.contains("could not") && !w.contains("cannot")), "{warnings:?}");
        assert_eq!(book.charts()[0].col, 3);
        let bytes = book.file_bytes(path.to_str().unwrap()).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut content = String::new();
        zip.by_name("content.xml").unwrap().read_to_string(&mut content).unwrap();
        let (found, warnings) = read(&mut zip, &content, &["Sheet1".into()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(found[0].chart.kind, ChartKind::Pie);
        assert_eq!(found[0].chart.series[0].categories.as_deref(), Some("Sheet1!$A$2:$A$4"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn pie_rotation_and_exploded_slices_refuse_lossy_ods_save_or_import() {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        for (cell, input) in [("A1", "Month"), ("B1", "Sales"), ("A2", "Jan"), ("B2", "12")] {
            session.execute_headless("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        session.execute_headless("insert.chart", json!({"range":"A1:B2", "type":"pie", "at":"D2"})).unwrap();
        let mut wb = (*session.doc().unwrap().wb).clone();
        wb.sheet_mut(0).unwrap().charts[0].first_slice_angle = 90;
        assert!(super::super::write(&wb).unwrap_err().contains("save as XLSX"));
        let xml = r#"<office:document-content><chart:chart chart:class="chart:circle"><chart:plot-area><chart:series chart:class="chart:circle" chart:values-cell-range-address="$Sheet1.B2:$Sheet1.B2"/></chart:plot-area></chart:chart></office:document-content>"#;
        assert!(unsupported_plot_semantics(&xml.replace("<chart:plot-area>", "<chart:plot-area chart:pie-offset=\"20\">")));
        assert!(unsupported_plot_semantics(&xml.replace("<chart:plot-area>", "<chart:plot-area chart:starting-angle=\"90\">")));
    }

    #[test]
    fn chart_without_gridlines_round_trips_without_adding_a_grid() {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        for (cell, input) in [("A1", "Month"), ("B1", "Sales"), ("A2", "Jan"), ("B2", "12"),
            ("A3", "Feb"), ("B3", "18")] {
            session.execute_headless("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        session.execute_headless("insert.chart", json!({"range":"A1:B3", "type":"column", "at":"D2"})).unwrap();
        let mut wb = (*session.doc().unwrap().wb).clone();
        wb.sheet_mut(0).unwrap().charts[0].gridlines = false;
        let bytes = super::super::write(&wb).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut content = String::new();
        zip.by_name("content.xml").unwrap().read_to_string(&mut content).unwrap();
        let mut object = String::new();
        zip.by_name("Object 1/content.xml").unwrap().read_to_string(&mut object).unwrap();
        assert!(!object.contains("<chart:grid"));
        let (found, warnings) = read(&mut zip, &content, &["Sheet1".into()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(found.len(), 1);
        assert!(!found[0].chart.gridlines);
    }

    #[test]
    fn unsupported_odf_plot_styles_refuse_chart_import() {
        use std::io::Write;
        let source = include_bytes!("../../assets/lo-charts.ods");
        for flag in ["vertical", "stacked", "percentage", "three-dimensional"] {
            for in_styles_file in [false, true] {
                let mut input = zip::ZipArchive::new(std::io::Cursor::new(source.as_slice())).unwrap();
                let output = std::io::Cursor::new(Vec::new());
                let mut writer = zip::ZipWriter::new(output);
                for index in 0..input.len() {
                    let mut entry = input.by_index(index).unwrap();
                    let name = entry.name().to_string();
                    if entry.is_dir() {
                        writer.add_directory(name, zip::write::SimpleFileOptions::default()).unwrap();
                        continue;
                    }
                    let mut data = Vec::new();
                    entry.read_to_end(&mut data).unwrap();
                    if name == if in_styles_file { "Object 1/styles.xml" } else { "Object 1/content.xml" } {
                        let xml = String::from_utf8(data).unwrap();
                        let xml = if in_styles_file {
                            xml.replace("</office:document-styles>", &format!("<office:automatic-styles><style:style style:name=\"unsupported\" style:family=\"chart\"><style:chart-properties chart:{flag}=\"true\"/></style:style></office:automatic-styles></office:document-styles>"))
                        } else {
                            xml.replacen("<style:chart-properties", &format!("<style:chart-properties chart:{flag}=\"true\""), 1)
                        };
                        data = xml.into_bytes();
                    }
                    writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                    writer.write_all(&data).unwrap();
                }
                let bytes = writer.finish().unwrap().into_inner();
                let extras = super::super::extras(&bytes).unwrap();
                assert_eq!(extras.charts.len(), 1, "{flag} in_styles_file={in_styles_file}");
                assert!(extras.chart_warnings.iter().any(|w| w.contains("cannot preserve")), "{flag} {in_styles_file}: {:?}", extras.chart_warnings);
            }
        }
    }

    #[test]
    fn libreoffice_resaved_chart_fixture_opens_with_positions_and_categories() {
        let path = std::env::temp_dir().join(format!("spreadeagle-lo-chart-{}.ods", std::process::id()));
        std::fs::write(&path, include_bytes!("../../assets/lo-charts.ods")).unwrap();
        let (mut book, warnings) = crate::book::Book::open(path.to_str().unwrap()).unwrap();
        assert!(warnings.iter().all(|w| !w.contains("could not") && !w.contains("cannot")), "{warnings:?}");
        assert_eq!(book.charts().len(), 2);
        assert_eq!(book.charts()[0].col, 3);
        assert!(book.charts()[0].row <= 1);
        assert_eq!(book.charts()[1].col, 3);
        assert!((18..=19).contains(&book.charts()[1].row));
        let bytes = book.file_bytes(path.to_str().unwrap()).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut xml = String::new();
        zip.by_name("content.xml").unwrap().read_to_string(&mut xml).unwrap();
        let (found, warnings) = read(&mut zip, &xml, &["Sheet1".into()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(found[0].chart.series[0].categories.as_deref(), Some("Sheet1!$A$2:$A$4"));
        std::fs::remove_file(path).unwrap();
    }
}
