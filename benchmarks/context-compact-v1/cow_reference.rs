use std::sync::Arc;
fn trim(mut xs: Arc<Vec<i64>>) -> Option<Arc<Vec<i64>>> {
    Arc::make_mut(&mut xs).pop()?;
    Some(xs)
}
