pub fn run(allowed: bool, blocked: bool, score: i64) -> bool {
    allowed && !blocked && (score >= 10 || score == -1)
}
