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

#[test]
fn test_trim_filename_multibyte() {
    // Byte-based slicing used to panic when a cut landed inside 'é'.
    let name = "aaaaaaaaaaaaéééééééééééééééééé.txt";
    let trimmed = cp2::utils::trim_filename(name, 28);
    assert_eq!(trimmed.chars().count(), 28);
    assert_eq!(trimmed, "aaaaaaaaaaaaé...éééééééé.txt");
    assert_eq!(cp2::utils::trim_filename("ééé", 3), "ééé");
}
