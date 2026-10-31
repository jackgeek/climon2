//! Throwaway #52 fixture. No terminal engine or native integration in the serializer.

use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub struct Cell {
    pub text: String,
    pub width: usize,
    pub blue: bool,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub soft_wrap: bool,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub columns: usize,
    pub normal: Vec<Row>,
    pub history: Vec<Row>,
    pub alternate: Option<Vec<Row>>,
}

pub fn restore(snapshot: &Snapshot) -> Result<Vec<u8>, String> {
    validate(snapshot)?;
    let height = snapshot.normal.len();
    let mut bytes =
        format!("\x1b[?1049l\x1b[0m\x1b[?6l\x1b[4l\x1b[?7h\x1b[r\x1b[{height};1H").into_bytes();
    for _ in 0..height {
        bytes.extend_from_slice(b"\r\n");
    }
    let rows: Vec<&Row> = snapshot.history.iter().chain(&snapshot.normal).collect();
    render_rows(&mut bytes, &rows);
    if let Some(alternate) = &snapshot.alternate {
        bytes.extend_from_slice(b"\x1b[?1049h\x1b[H");
        render_rows(&mut bytes, &alternate.iter().collect::<Vec<_>>());
    }
    bytes.extend_from_slice(b"\x1b[0m");
    Ok(bytes)
}

fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if !(2..=256).contains(&snapshot.columns)
        || !(2..=64).contains(&snapshot.normal.len())
        || snapshot.history.len() > 128
    {
        return Err("fixture geometry exceeds its finite bounds".into());
    }
    if let Some(alternate) = &snapshot.alternate
        && alternate.len() != snapshot.normal.len()
    {
        return Err("alternate and normal screen heights differ".into());
    }
    for rows in [
        Some(&snapshot.history),
        Some(&snapshot.normal),
        snapshot.alternate.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        for row in rows {
            validate_row(row, snapshot.columns)?;
        }
    }
    for screen in [Some(&snapshot.normal), snapshot.alternate.as_ref()]
        .into_iter()
        .flatten()
    {
        if screen.last().is_some_and(|row| row.soft_wrap) {
            return Err("final screen row cannot continue beyond this fixture".into());
        }
    }
    Ok(())
}

fn validate_row(row: &Row, columns: usize) -> Result<(), String> {
    if row.cells.len() > columns {
        return Err("too many cells in fixture row".into());
    }
    for cell in &row.cells {
        if cell.text.is_empty()
            || cell.text.len() > 32
            || cell.text.chars().any(char::is_control)
            || !(1..=2).contains(&cell.width)
            || cell.text.width() != cell.width
        {
            return Err("invalid printable fixture cell or declared width".into());
        }
    }
    let width: usize = row.cells.iter().map(|cell| cell.width).sum();
    if width > columns || (row.soft_wrap && width != columns) {
        return Err("row width is inconsistent with supplied wrap provenance".into());
    }
    Ok(())
}

fn render_rows(bytes: &mut Vec<u8>, rows: &[&Row]) {
    for (index, row) in rows.iter().enumerate() {
        for cell in &row.cells {
            bytes.extend_from_slice(if cell.blue { b"\x1b[34m" } else { b"\x1b[0m" });
            bytes.extend_from_slice(cell.text.as_bytes());
        }
        bytes.extend_from_slice(b"\x1b[0m");
        if index + 1 < rows.len() && !row.soft_wrap {
            bytes.extend_from_slice(b"\r\n");
        }
    }
}

pub fn row(text: &str) -> Row {
    Row {
        cells: text
            .chars()
            .map(|c| Cell {
                text: c.to_string(),
                width: 1,
                blue: false,
            })
            .collect(),
        soft_wrap: false,
    }
}

pub fn sample() -> Snapshot {
    let mut soft = row("SOFT01234567890123456789");
    soft.soft_wrap = true;
    let unicode = Row {
        cells: vec![
            Cell {
                text: "界".into(),
                width: 2,
                blue: true,
            },
            Cell {
                text: "e\u{301}".into(),
                width: 1,
                blue: false,
            },
        ],
        soft_wrap: false,
    };
    Snapshot {
        columns: 24,
        normal: (0..6).map(|n| row(&format!("NORMAL-{n}"))).collect(),
        history: vec![row("HARD-A"), soft, row("tail"), unicode, row("HARD-Z")],
        alternate: None,
    }
}
