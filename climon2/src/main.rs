fn main() {
    let mut buffer = itoa::Buffer::new();
    println!("hello from climon2 {}", buffer.format(42));
}
