fn main() {
    let data = vec![1, 2, 3];
    std::thread::spawn(move || drop(data)).join().unwrap();
    println!("{}", data.len());
}
