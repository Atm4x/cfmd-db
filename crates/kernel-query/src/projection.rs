use super::{RelQueryError, Row};

pub(super) fn project_rows(rows: Vec<Row>, columns: &[usize]) -> Result<Vec<Row>, RelQueryError> {
    rows.into_iter()
        .map(|row| project_row(&row, columns))
        .collect()
}

pub(super) fn project_row(row: &Row, columns: &[usize]) -> Result<Row, RelQueryError> {
    columns
        .iter()
        .map(|column| {
            row.get(*column)
                .cloned()
                .ok_or(RelQueryError::ColumnOutOfBounds)
        })
        .collect()
}
