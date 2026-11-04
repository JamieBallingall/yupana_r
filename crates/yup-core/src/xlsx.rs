//! Step 8, Package: the workbook's parts as XML, in a zip.
//!
//! The parts are the fewest the format needs (ECMA-376 Part 1, §12.3 and §18): the workbook, a
//! part for each sheet, the styles, the shared strings when there is text, and, when there are
//! dynamic array formulas, the metadata part that marks them as such ([MS-XLSX]). Elements come
//! in the order their schema gives, because the spreadsheet app offers to repair a file whose
//! elements are out of order.
//!
//! A dynamic array formula is written in its anchor cell as an array formula whose range is
//! the anchor alone, with the cell metadata that marks it dynamic. No values are cached: the
//! workbook asks to be calculated in full when it is opened, and the formula then spills.

use std::collections::{BTreeMap, BTreeSet};

use crate::cells::cell_name;
use crate::lower::{Cell, DEFAULT_FONT, DEFAULT_SIZE, SheetCells, Value, Workbook, number};
use crate::style::{Alignment, Border, CellStyle, Font, Rgb};
use crate::zip;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE_RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CONTENT_TYPES: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const DYNAMIC_ARRAY: &str = "http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray";
/// The extension that carries a dynamic array's properties, as [MS-XLSX] names it.
const DYNAMIC_ARRAY_EXTENSION: &str = "{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}";
const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
/// The first identifier free for a number format of the file's own; lower ones are built in.
const FIRST_FORMAT_ID: usize = 164;

fn content_type(kind: &str) -> String {
    format!("application/vnd.openxmlformats-officedocument.spreadsheetml.{kind}+xml")
}

fn relationship_type(kind: &str) -> String {
    format!("{RELATIONSHIPS}/{kind}")
}

/// Escapes text for XML content and attribute values alike.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// The workbook as the bytes of an xlsx file.
pub fn package(workbook: &Workbook) -> Vec<u8> {
    let parts: Vec<(String, Vec<u8>)> = parts(workbook)
        .into_iter()
        .map(|(name, xml)| (name, xml.into_bytes()))
        .collect();
    zip::store(&parts)
}

/// The workbook's parts, each a path in the package and its XML.
pub fn parts(workbook: &Workbook) -> Vec<(String, String)> {
    let strings = SharedStrings::of(workbook);
    let has_formulas = workbook
        .sheets
        .iter()
        .flat_map(|sheet| sheet.cells.values())
        .any(|cell| matches!(cell.value, Value::Formula(_)));
    let contents = Contents {
        sheets: workbook.sheets.len(),
        strings: !strings.list.is_empty(),
        metadata: has_formulas,
    };
    let mut parts = vec![
        ("[Content_Types].xml".to_string(), contents.types()),
        ("_rels/.rels".to_string(), root_relationships()),
        ("xl/workbook.xml".to_string(), workbook_xml(workbook)),
        (
            "xl/_rels/workbook.xml.rels".to_string(),
            contents.relationships(),
        ),
        (
            "xl/styles.xml".to_string(),
            styles_xml(&workbook.styles, &workbook.font),
        ),
    ];
    if contents.strings {
        parts.push(("xl/sharedStrings.xml".to_string(), strings.xml()));
    }
    if contents.metadata {
        parts.push(("xl/metadata.xml".to_string(), metadata_xml()));
    }
    let active = active_sheet(workbook);
    for (index, sheet) in workbook.sheets.iter().enumerate() {
        parts.push((
            format!("xl/worksheets/sheet{}.xml", index + 1),
            sheet_xml(sheet, index == active, &strings),
        ));
    }
    parts
}

/// The parts as text, one element to a line, for reading what Package produced.
pub fn listing(workbook: &Workbook) -> String {
    let mut out = String::new();
    for (name, xml) in parts(workbook) {
        out.push_str(&format!("=== {name}\n"));
        out.push_str(&xml.replace("><", ">\n<"));
        out.push('\n');
    }
    out
}

/// Which optional parts the package has.
struct Contents {
    sheets: usize,
    strings: bool,
    metadata: bool,
}

impl Contents {
    fn types(&self) -> String {
        let mut x = format!("{DECLARATION}<Types xmlns=\"{CONTENT_TYPES}\">");
        x.push_str(
            "<Default Extension=\"rels\" \
             ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
        );
        x.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
        let mut part = |name: &str, kind: &str| {
            x.push_str(&format!(
                "<Override PartName=\"{name}\" ContentType=\"{}\"/>",
                content_type(kind)
            ));
        };
        part("/xl/workbook.xml", "sheet.main");
        for index in 1..=self.sheets {
            part(&format!("/xl/worksheets/sheet{index}.xml"), "worksheet");
        }
        part("/xl/styles.xml", "styles");
        if self.strings {
            part("/xl/sharedStrings.xml", "sharedStrings");
        }
        if self.metadata {
            part("/xl/metadata.xml", "sheetMetadata");
        }
        x.push_str("</Types>");
        x
    }

    fn relationships(&self) -> String {
        let mut x = format!("{DECLARATION}<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS}\">");
        let mut id = 0;
        let mut relationship = |kind: &str, target: &str| {
            id += 1;
            x.push_str(&format!(
                "<Relationship Id=\"rId{id}\" Type=\"{}\" Target=\"{target}\"/>",
                relationship_type(kind)
            ));
        };
        // The sheets come first, so that sheet n has the relationship rIdn.
        for index in 1..=self.sheets {
            relationship("worksheet", &format!("worksheets/sheet{index}.xml"));
        }
        relationship("styles", "styles.xml");
        if self.strings {
            relationship("sharedStrings", "sharedStrings.xml");
        }
        if self.metadata {
            relationship("sheetMetadata", "metadata.xml");
        }
        x.push_str("</Relationships>");
        x
    }
}

fn root_relationships() -> String {
    format!(
        "{DECLARATION}<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS}\"><Relationship Id=\"rId1\" \
         Type=\"{}\" Target=\"xl/workbook.xml\"/></Relationships>",
        relationship_type("officeDocument")
    )
}

/// The sheet the workbook opens on: the first that is not hidden.
fn active_sheet(workbook: &Workbook) -> usize {
    workbook
        .sheets
        .iter()
        .position(|sheet| !sheet.hidden)
        .expect("Resolve keeps at least one sheet shown")
}

fn workbook_xml(workbook: &Workbook) -> String {
    let mut x = format!("{DECLARATION}<workbook xmlns=\"{MAIN}\" xmlns:r=\"{RELATIONSHIPS}\">");
    let active = match active_sheet(workbook) {
        0 => String::new(),
        index => format!(" activeTab=\"{index}\""),
    };
    x.push_str(&format!(
        "<bookViews><workbookView xWindow=\"0\" yWindow=\"0\" windowWidth=\"16000\" \
         windowHeight=\"9000\"{active}/></bookViews>"
    ));
    x.push_str("<sheets>");
    for (index, sheet) in workbook.sheets.iter().enumerate() {
        let state = if sheet.hidden {
            " state=\"hidden\""
        } else {
            ""
        };
        x.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{}\"{state} r:id=\"rId{}\"/>",
            escape(&sheet.name),
            index + 1,
            index + 1
        ));
    }
    x.push_str("</sheets>");
    if !workbook.names.is_empty() {
        x.push_str("<definedNames>");
        for name in &workbook.names {
            x.push_str(&format!(
                "<definedName name=\"{}\">{}</definedName>",
                escape(&name.name),
                escape(&name.refers_to)
            ));
        }
        x.push_str("</definedNames>");
    }
    // Nothing is cached, so the workbook asks to be calculated in full when it is opened.
    x.push_str("<calcPr fullCalcOnLoad=\"1\"/>");
    x.push_str("</workbook>");
    x
}

fn position<T: PartialEq>(list: &mut Vec<T>, item: T) -> usize {
    match list.iter().position(|known| *known == item) {
        Some(index) => index,
        None => {
            list.push(item);
            list.len() - 1
        }
    }
}

/// A font as the file writes it, with the workbook's font for what the style leaves out
/// (ECMA-376 Part 1, §18.8).
fn font_xml(font: &Font, default: &Font) -> String {
    let mut x = String::from("<font>");
    if font.bold == Some(true) {
        x.push_str("<b/>");
    }
    if font.italic == Some(true) {
        x.push_str("<i/>");
    }
    let size = font.size.or(default.size).unwrap_or(DEFAULT_SIZE);
    x.push_str(&format!("<sz val=\"{}\"/>", number(size)));
    if let Some(color) = font.color {
        x.push_str(&format!("<color rgb=\"{}\"/>", color.argb()));
    }
    let name = font
        .name
        .as_deref()
        .or(default.name.as_deref())
        .unwrap_or(DEFAULT_FONT);
    x.push_str(&format!("<name val=\"{}\"/></font>", escape(name)));
    x
}

/// A solid fill of one color.
fn fill_xml(color: Rgb) -> String {
    format!(
        "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"{}\"/></patternFill></fill>",
        color.argb()
    )
}

/// A border, its sides in the order the schema gives them.
fn border_xml(border: &Border) -> String {
    let mut x = String::from("<border>");
    for (name, edge) in [
        ("left", &border.left),
        ("right", &border.right),
        ("top", &border.top),
        ("bottom", &border.bottom),
    ] {
        match edge {
            None => x.push_str(&format!("<{name}/>")),
            Some(edge) => {
                x.push_str(&format!("<{name}"));
                if let Some(weight) = edge.weight {
                    x.push_str(&format!(" style=\"{weight}\""));
                }
                x.push('>');
                if let Some(color) = edge.color {
                    x.push_str(&format!("<color rgb=\"{}\"/>", color.argb()));
                }
                x.push_str(&format!("</{name}>"));
            }
        }
    }
    x.push_str("<diagonal/></border>");
    x
}

/// A cell's alignment, if it has one.
fn alignment_xml(align: &Alignment) -> Option<String> {
    let mut attributes = String::new();
    if let Some(horizontal) = align.horizontal {
        attributes.push_str(&format!(" horizontal=\"{horizontal}\""));
    }
    if let Some(vertical) = align.vertical {
        attributes.push_str(&format!(" vertical=\"{vertical}\""));
    }
    if let Some(wrap) = align.wrap {
        attributes.push_str(&format!(" wrapText=\"{}\"", u8::from(wrap)));
    }
    if let Some(indent) = align.indent {
        attributes.push_str(&format!(" indent=\"{indent}\""));
    }
    (!attributes.is_empty()).then(|| format!("<alignment{attributes}/>"))
}

fn styles_xml(styles: &[CellStyle], default: &Font) -> String {
    // The workbook's own font comes first, as font 0, and the two reserved fills and an empty
    // border likewise, so that a cell with no style of its own takes them.
    let mut fonts = vec![font_xml(&Font::default(), default)];
    let mut fills: Vec<String> = Vec::new();
    let mut borders = vec![border_xml(&Border::default())];
    let mut formats: Vec<&str> = Vec::new();
    let mut xfs = String::new();
    for style in styles {
        let font = position(&mut fonts, font_xml(&style.font, default));
        // The first two fills are the ones the format reserves.
        let fill = match style.fill {
            Some(color) => 2 + position(&mut fills, fill_xml(color)),
            None => 0,
        };
        let border = position(&mut borders, border_xml(&style.border));
        let format = match &style.format {
            Some(format) => FIRST_FORMAT_ID + position(&mut formats, format.as_str()),
            None => 0,
        };
        let mut applied = String::new();
        for (id, attribute) in [
            (format, "applyNumberFormat"),
            (font, "applyFont"),
            (fill, "applyFill"),
            (border, "applyBorder"),
        ] {
            if id != 0 {
                applied.push_str(&format!(" {attribute}=\"1\""));
            }
        }
        let alignment = alignment_xml(&style.align);
        if alignment.is_some() {
            applied.push_str(" applyAlignment=\"1\"");
        }
        let head = format!(
            "<xf numFmtId=\"{format}\" fontId=\"{font}\" fillId=\"{fill}\" borderId=\"{border}\" \
             xfId=\"0\"{applied}"
        );
        match alignment {
            Some(alignment) => xfs.push_str(&format!("{head}>{alignment}</xf>")),
            None => xfs.push_str(&format!("{head}/>")),
        }
    }
    let mut x = format!("{DECLARATION}<styleSheet xmlns=\"{MAIN}\">");
    if !formats.is_empty() {
        x.push_str(&format!("<numFmts count=\"{}\">", formats.len()));
        for (index, format) in formats.iter().enumerate() {
            x.push_str(&format!(
                "<numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
                FIRST_FORMAT_ID + index,
                escape(format)
            ));
        }
        x.push_str("</numFmts>");
    }
    x.push_str(&format!("<fonts count=\"{}\">", fonts.len()));
    for font in &fonts {
        x.push_str(font);
    }
    x.push_str("</fonts>");
    x.push_str(&format!(
        "<fills count=\"{}\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill \
         patternType=\"gray125\"/></fill>",
        2 + fills.len()
    ));
    for fill in &fills {
        x.push_str(fill);
    }
    x.push_str("</fills>");
    x.push_str(&format!("<borders count=\"{}\">", borders.len()));
    for border in &borders {
        x.push_str(border);
    }
    x.push_str("</borders>");
    x.push_str(
        "<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" \
         borderId=\"0\"/></cellStyleXfs>",
    );
    x.push_str(&format!(
        "<cellXfs count=\"{}\">{xfs}</cellXfs>",
        styles.len()
    ));
    x.push_str(
        "<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" \
         builtinId=\"0\"/></cellStyles>",
    );
    x.push_str("</styleSheet>");
    x
}

/// Every distinct text in the workbook, in the order it first appears.
struct SharedStrings {
    list: Vec<String>,
    index: BTreeMap<String, usize>,
    /// How many cells hold text.
    uses: usize,
}

impl SharedStrings {
    fn of(workbook: &Workbook) -> SharedStrings {
        let mut strings = SharedStrings {
            list: Vec::new(),
            index: BTreeMap::new(),
            uses: 0,
        };
        for cell in workbook
            .sheets
            .iter()
            .flat_map(|sheet| sheet.cells.values())
        {
            if let Value::Text(text) = &cell.value {
                strings.uses += 1;
                if !strings.index.contains_key(text) {
                    strings.index.insert(text.clone(), strings.list.len());
                    strings.list.push(text.clone());
                }
            }
        }
        strings
    }

    fn xml(&self) -> String {
        let mut x = format!(
            "{DECLARATION}<sst xmlns=\"{MAIN}\" count=\"{}\" uniqueCount=\"{}\">",
            self.uses,
            self.list.len()
        );
        for text in &self.list {
            // Without this, a reader may drop spaces at either end and line breaks.
            let space = if text.starts_with(char::is_whitespace)
                || text.ends_with(char::is_whitespace)
                || text.contains('\n')
            {
                " xml:space=\"preserve\""
            } else {
                ""
            };
            x.push_str(&format!("<si><t{space}>{}</t></si>", escape(text)));
        }
        x.push_str("</sst>");
        x
    }
}

/// The metadata that marks a formula as a dynamic array: one metadata type, one block of
/// dynamic array properties, and one cell metadata record that cells point at with `cm="1"`.
fn metadata_xml() -> String {
    format!(
        "{DECLARATION}<metadata xmlns=\"{MAIN}\" xmlns:xda=\"{DYNAMIC_ARRAY}\"><metadataTypes \
         count=\"1\"><metadataType name=\"XLDAPR\" minSupportedVersion=\"120000\" copy=\"1\" \
         pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" \
         clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\" \
         cellMeta=\"1\"/></metadataTypes><futureMetadata name=\"XLDAPR\" \
         count=\"1\"><bk><extLst><ext uri=\"{DYNAMIC_ARRAY_EXTENSION}\"><xda:dynamicArrayProperties \
         fDynamic=\"1\" fCollapsed=\"0\"/></ext></extLst></bk></futureMetadata><cellMetadata \
         count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata></metadata>"
    )
}

fn sheet_xml(sheet: &SheetCells, active: bool, strings: &SharedStrings) -> String {
    let mut x = format!("{DECLARATION}<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{RELATIONSHIPS}\">");
    // The sheet's properties come first in the worksheet, and its tab's color among them.
    if let Some(color) = sheet.color {
        x.push_str(&format!(
            "<sheetPr><tabColor rgb=\"{}\"/></sheetPr>",
            color.argb()
        ));
    }
    let selected = if active { " tabSelected=\"1\"" } else { "" };
    x.push_str(&format!(
        "<sheetViews><sheetView{selected} workbookViewId=\"0\"/></sheetViews>"
    ));
    x.push_str("<sheetFormatPr defaultRowHeight=\"15\"/>");
    let columns: BTreeSet<u32> = sheet
        .widths
        .keys()
        .chain(sheet.column_styles.keys())
        .copied()
        .collect();
    if !columns.is_empty() {
        x.push_str("<cols>");
        for column in columns {
            x.push_str(&format!("<col min=\"{column}\" max=\"{column}\""));
            if let Some(width) = sheet.widths.get(&column) {
                x.push_str(&format!(" width=\"{width:.2}\" customWidth=\"1\""));
            }
            if let Some(style) = sheet.column_styles.get(&column) {
                x.push_str(&format!(" style=\"{style}\""));
            }
            x.push_str("/>");
        }
        x.push_str("</cols>");
    }
    x.push_str("<sheetData>");
    let rows: BTreeSet<u32> = sheet
        .cells
        .keys()
        .map(|&(row, _)| row)
        .chain(sheet.row_styles.keys().copied())
        .collect();
    for r in rows {
        match sheet.row_styles.get(&r) {
            Some(style) => x.push_str(&format!("<row r=\"{r}\" s=\"{style}\" customFormat=\"1\">")),
            None => x.push_str(&format!("<row r=\"{r}\">")),
        }
        for (&(_, c), cell) in sheet.cells.range((r, 0)..=(r, u32::MAX)) {
            x.push_str(&cell_xml(cell_name(r, c), cell, strings));
        }
        x.push_str("</row>");
    }
    x.push_str("</sheetData>");
    x.push_str(
        "<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" \
         footer=\"0.3\"/>",
    );
    x.push_str("</worksheet>");
    x
}

fn cell_xml(name: String, cell: &Cell, strings: &SharedStrings) -> String {
    // An empty cell is written only to give it a style, so it says its style even when that is
    // the default.
    let style = if cell.style == 0 && cell.value != Value::Empty {
        String::new()
    } else {
        format!(" s=\"{}\"", cell.style)
    };
    match &cell.value {
        Value::Number(value) => format!("<c r=\"{name}\"{style}><v>{}</v></c>", number(*value)),
        Value::Text(text) => format!(
            "<c r=\"{name}\"{style} t=\"s\"><v>{}</v></c>",
            strings.index[text]
        ),
        Value::Bool(value) => format!(
            "<c r=\"{name}\"{style} t=\"b\"><v>{}</v></c>",
            u8::from(*value)
        ),
        Value::Formula(formula) => format!(
            "<c r=\"{name}\"{style} cm=\"1\"><f t=\"array\" ref=\"{name}\">{}</f></c>",
            escape(formula)
        ),
        Value::Empty => format!("<c r=\"{name}\"{style}/>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::DefinedName;

    /// Checks that `xml` is well formed: one root, tags nested and closed, attributes quoted,
    /// and only the five predefined entities.
    fn well_formed(xml: &str) -> Result<(), String> {
        let body = xml.strip_prefix(DECLARATION).ok_or("no XML declaration")?;
        let mut open: Vec<&str> = Vec::new();
        let mut roots = 0;
        let mut rest = body;
        while let Some(start) = rest.find(['<', '&']) {
            let (text, tail) = rest.split_at(start);
            if !text.trim().is_empty() && open.is_empty() {
                return Err(format!("text outside the root: {text}"));
            }
            if let Some(tail) = tail.strip_prefix('&') {
                let end = tail.find(';').ok_or("an unterminated entity")?;
                if !["amp", "lt", "gt", "quot", "apos"].contains(&&tail[..end]) {
                    return Err(format!("an unknown entity &{};", &tail[..end]));
                }
                rest = &tail[end + 1..];
                continue;
            }
            let end = tail.find('>').ok_or("an unterminated tag")?;
            let tag = &tail[1..end];
            rest = &tail[end + 1..];
            if let Some(name) = tag.strip_prefix('/') {
                if open.pop() != Some(name) {
                    return Err(format!("</{name}> closes the wrong element"));
                }
                continue;
            }
            let empty = tag.ends_with('/');
            let tag = tag.trim_end_matches('/');
            let name = tag.split(' ').next().unwrap_or_default();
            if !tag.matches('"').count().is_multiple_of(2) || tag.contains('<') {
                return Err(format!("a malformed tag <{tag}>"));
            }
            if open.is_empty() {
                roots += 1;
            }
            if !empty {
                open.push(name);
            }
        }
        if !open.is_empty() || roots != 1 {
            return Err(format!("unclosed {open:?}, {roots} roots"));
        }
        Ok(())
    }

    fn workbook() -> Workbook {
        let mut cells = BTreeMap::new();
        let text = |t: &str| Value::Text(t.to_string());
        cells.insert(
            (2, 2),
            Cell {
                value: text("Rates & <names>"),
                style: 0,
            },
        );
        cells.insert(
            (2, 4),
            Cell {
                value: Value::Number(0.25),
                style: 1,
            },
        );
        cells.insert(
            (3, 2),
            Cell {
                value: text(" padded "),
                style: 0,
            },
        );
        cells.insert(
            (3, 4),
            Cell {
                value: Value::Formula("IF(a<b,\"x\",b)".to_string()),
                style: 0,
            },
        );
        let mut styled = CellStyle {
            format: Some("0.0%\"x\"".to_string()),
            fill: Some(Rgb {
                red: 217,
                green: 217,
                blue: 217,
            }),
            ..CellStyle::default()
        };
        styled.font.bold = Some(true);
        styled.border.top = Some(crate::style::Edge {
            weight: Some("thin"),
            color: None,
        });
        styled.align.horizontal = Some("center");
        Workbook {
            sheets: vec![SheetCells {
                name: "Rates & 'more'".to_string(),
                color: Some(Rgb {
                    red: 255,
                    green: 0,
                    blue: 0,
                }),
                hidden: false,
                cells,
                row_styles: BTreeMap::from([(3, 1), (5, 1)]),
                column_styles: BTreeMap::from([(7, 1)]),
                widths: BTreeMap::from([(2, 20.0)]),
            }],
            names: vec![DefinedName {
                name: "rate".to_string(),
                refers_to: "'Rates & ''more'''!$D$2".to_string(),
            }],
            styles: vec![CellStyle::default(), styled],
            font: Font {
                name: Some(DEFAULT_FONT.to_string()),
                size: Some(DEFAULT_SIZE),
                ..Font::default()
            },
        }
    }

    #[test]
    fn every_part_is_well_formed_and_listed_in_the_content_types() {
        let parts = parts(&workbook());
        let names: Vec<&str> = parts.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "xl/workbook.xml",
                "xl/_rels/workbook.xml.rels",
                "xl/styles.xml",
                "xl/sharedStrings.xml",
                "xl/metadata.xml",
                "xl/worksheets/sheet1.xml",
            ]
        );
        for (name, xml) in &parts {
            assert_eq!(well_formed(xml), Ok(()), "{name}");
        }
        let types = &parts[0].1;
        for name in &names[2..] {
            if !name.ends_with(".rels") {
                assert!(types.contains(&format!("PartName=\"/{name}\"")), "{name}");
            }
        }
    }

    #[test]
    fn text_is_escaped_and_shared() {
        let parts = parts(&workbook());
        let find = |name: &str| &parts.iter().find(|(n, _)| n == name).expect("a part").1;
        assert!(find("xl/sharedStrings.xml").contains(
            "<si><t>Rates &amp; &lt;names&gt;</t></si><si><t xml:space=\"preserve\"> padded \
             </t></si>"
        ));
        assert!(find("xl/workbook.xml").contains(
            "<sheet name=\"Rates &amp; &apos;more&apos;\" sheetId=\"1\" r:id=\"rId1\"/>"
        ));
        let sheet = find("xl/worksheets/sheet1.xml");
        assert!(sheet.contains(
            "<c r=\"D3\" cm=\"1\"><f t=\"array\" ref=\"D3\">IF(a&lt;b,&quot;x&quot;,b)</f></c>"
        ));
        assert!(sheet.contains("<c r=\"D2\" s=\"1\"><v>0.25</v></c>"));
        assert!(sheet.contains("<row r=\"3\" s=\"1\" customFormat=\"1\"><c r=\"B3\""));
        assert!(sheet.contains("<row r=\"5\" s=\"1\" customFormat=\"1\"></row></sheetData>"));
        assert!(
            find("xl/styles.xml")
                .contains("<numFmt numFmtId=\"164\" formatCode=\"0.0%&quot;x&quot;\"/>")
        );
        assert!(sheet.starts_with(&format!(
            "{DECLARATION}<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{RELATIONSHIPS}\"><sheetPr><tabColor \
             rgb=\"FFFF0000\"/></sheetPr>"
        )));
        assert!(sheet.contains(
            "<cols><col min=\"2\" max=\"2\" width=\"20.00\" customWidth=\"1\"/><col min=\"7\" \
             max=\"7\" style=\"1\"/></cols>"
        ));
    }

    #[test]
    fn styles_write_fonts_fills_borders_and_alignment() {
        let styles = styles_xml(&workbook().styles, &workbook().font);
        for part in [
            "<fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz \
             val=\"11\"/><name val=\"Calibri\"/></font></fonts>",
            "<fills count=\"3\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill \
             patternType=\"gray125\"/></fill><fill><patternFill patternType=\"solid\"><fgColor \
             rgb=\"FFD9D9D9\"/></patternFill></fill></fills>",
            "<border><left/><right/><top style=\"thin\"></top><bottom/><diagonal/></border>",
            "<xf numFmtId=\"164\" fontId=\"1\" fillId=\"2\" borderId=\"1\" xfId=\"0\" \
             applyNumberFormat=\"1\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\" \
             applyAlignment=\"1\"><alignment horizontal=\"center\"/></xf>",
        ] {
            assert!(styles.contains(part), "{part}\n{styles}");
        }
    }

    #[test]
    fn the_package_is_a_zip_of_the_parts() {
        let workbook = workbook();
        let read = zip::read(&package(&workbook));
        let expected: Vec<(String, Vec<u8>)> = parts(&workbook)
            .into_iter()
            .map(|(name, xml)| (name, xml.into_bytes()))
            .collect();
        assert_eq!(read, expected);
    }
}
