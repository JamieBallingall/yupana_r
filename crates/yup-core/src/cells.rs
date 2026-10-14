//! Cells and sheets as the workbook names them: A1 notation, and quoted sheet names.
//!
//! Lower is the only step that writes these; before it, a cell is a pair of numbers.

/// The name of a column: 1 is `A`, 26 is `Z`, 27 is `AA`.
pub fn column_name(column: u32) -> String {
    let mut letters = Vec::new();
    let mut rest = column;
    while rest > 0 {
        let digit = u8::try_from((rest - 1) % 26).expect("a remainder of 26 fits in a byte");
        letters.push(char::from(b'A' + digit));
        rest = (rest - 1) / 26;
    }
    letters.iter().rev().collect()
}

/// A cell in A1 notation, such as `D3`.
pub fn cell_name(row: u32, column: u32) -> String {
    format!("{}{row}", column_name(column))
}

/// A cell in A1 notation, fixed against copying, such as `$D$3`.
pub fn absolute(row: u32, column: u32) -> String {
    format!("${}${row}", column_name(column))
}

/// A sheet name as a formula refers to it: as it is when it is a plain name, quoted otherwise.
pub fn sheet_reference(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !reads_as_reference(&name.to_ascii_lowercase());
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Whether a lower-case name could be read as a reference or a logical: a cell (`tax2026`),
/// R1C1 notation (`r1c1`), `true` or `false`.
fn reads_as_reference(name: &str) -> bool {
    looks_like_cell(name) || looks_like_r1c1(name) || name == "true" || name == "false"
}

/// Whether a lower-case name reads as an A1 cell reference: one to three letters, then digits.
pub fn looks_like_cell(name: &str) -> bool {
    let letters = name.bytes().take_while(u8::is_ascii_lowercase).count();
    let digits = &name[letters..];
    (1..=3).contains(&letters) && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Whether a lower-case name reads as an R1C1 reference: `r`, `c`, `rc`, or either with digits.
pub fn looks_like_r1c1(name: &str) -> bool {
    let bytes = name.as_bytes();
    let digits_from = |mut i: usize| {
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        i
    };
    let row = bytes.first() == Some(&b'r');
    let mut i = if row { digits_from(1) } else { 0 };
    if bytes.get(i) == Some(&b'c') {
        i = digits_from(i + 1);
    } else if !row {
        return false;
    }
    i == bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_named_in_base_26_without_a_zero() {
        let names: Vec<String> = [1, 4, 26, 27, 52, 703, 16_384]
            .into_iter()
            .map(column_name)
            .collect();
        assert_eq!(names, ["A", "D", "Z", "AA", "AZ", "AAA", "XFD"]);
        assert_eq!(cell_name(3, 4), "D3");
        assert_eq!(absolute(3, 4), "$D$3");
    }

    #[test]
    fn sheet_names_are_quoted_when_needed() {
        assert_eq!(sheet_reference("Forecast"), "Forecast");
        assert_eq!(sheet_reference("Sheet1"), "Sheet1");
        assert_eq!(sheet_reference("Cash flow"), "'Cash flow'");
        assert_eq!(sheet_reference("Year's end"), "'Year''s end'");
        assert_eq!(sheet_reference("FY2026"), "'FY2026'");
        assert_eq!(sheet_reference("R1C1"), "'R1C1'");
        assert_eq!(sheet_reference("2026"), "'2026'");
    }

    #[test]
    fn names_that_read_as_references_are_recognized() {
        assert!(looks_like_cell("tax2026"));
        assert!(!looks_like_cell("taxes2026"));
        assert!(looks_like_r1c1("r1c1") && looks_like_r1c1("c") && looks_like_r1c1("rc"));
        assert!(!looks_like_r1c1("rate") && !looks_like_r1c1("cost"));
    }
}
