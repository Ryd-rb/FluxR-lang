pub fn greeting() -> Vec<u64> {
    "Hello café !\n".bytes().map(u64::from).collect()
}
