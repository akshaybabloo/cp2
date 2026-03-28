#[test]
fn test_trim_filename() {
    assert_eq!(cp2::utils::trim_filename("short.txt", 20), "short.txt");
    assert_eq!(
        cp2::utils::trim_filename("this_is_a_very_long_filename.txt", 20),
        "this_is_a...name.txt"
    );
    assert_eq!(cp2::utils::trim_filename("medium_length_name.txt", 10), "medi...txt");
    assert_eq!(cp2::utils::trim_filename("tiny", 3), "...");
}
