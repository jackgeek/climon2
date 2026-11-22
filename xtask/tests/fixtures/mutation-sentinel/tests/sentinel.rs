#[test]
fn only_the_checked_result_is_checked() {
    assert_eq!(mutation_sentinel::checked(1), 2);
    mutation_sentinel::unchecked(1);
}
