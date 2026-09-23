//! Dense GPU columns indexed by engine-declared table identities.
use super::RuntimeError;
use fresco_artifact::ManifestRoot;
use std::collections::BTreeSet;
fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::PassPlan(message.into())
}
pub fn column_bytes(
    manifest: &ManifestRoot,
    table_name: &str,
    column_name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, RuntimeError> {
    let table = manifest
        .tables
        .iter()
        .find(|t| t.name == table_name)
        .ok_or_else(|| invalid("recipe table is missing"))?;
    let column = table
        .fields
        .iter()
        .position(|c| c == column_name)
        .ok_or_else(|| invalid("recipe table column is missing"))?;
    let index_column = table
        .fields
        .iter()
        .position(|f| f == &table.index_field)
        .ok_or_else(|| invalid("table index field missing"))?;
    let count = table
        .records
        .iter()
        .map(|r| u64::from(r.index) + 1)
        .max()
        .unwrap_or(1);
    let size = count
        .checked_mul(4)
        .ok_or_else(|| invalid("table size overflow"))?;
    if size > max_bytes {
        return Err(invalid("recipe table exceeds device limits"));
    }
    let mut bytes = vec![0; usize::try_from(size).map_err(|_| invalid("table too large"))?];
    let mut ids = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for record in &table.records {
        if record.values.len() != table.fields.len()
            || record.values[index_column] != record.index
            || record.index < table.first_index
        {
            return Err(invalid("inconsistent table identity or record width"));
        }
        if !ids.insert(record.index) || !keys.insert(&record.key) {
            return Err(invalid("duplicate table index or key"));
        }
        let value = record
            .values
            .get(column)
            .ok_or_else(|| invalid("incomplete table record"))?;
        let offset =
            usize::try_from(record.index).map_err(|_| invalid("table index too large"))? * 4;
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}
