fn main() {
    let mut buffer = itoa::Buffer::new();
    println!("goodbye from climon2 {}", buffer.format(42));
}
