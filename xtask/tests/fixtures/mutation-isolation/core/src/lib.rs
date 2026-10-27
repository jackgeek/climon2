pub fn mutation_probe(value: i32) -> bool {
    value > 0 && value < 10
}

pub fn question_probe(value: Option<u32>) -> Option<u32> {
    Some(value? + 1)
}
