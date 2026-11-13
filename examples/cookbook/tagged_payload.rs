pub enum Choice {
    Empty,
    Number(i64),
    Other(i64),
}
pub fn run(value: Choice) -> Option<i64> {
    match value {
        Choice::Number(value) => Some(value),
        Choice::Empty | Choice::Other(_) => None,
    }
}
