use crate::logging::*;

#[test]
fn log_dir_path_includes_respondami_dir() {
    let path = log_dir_path();
    assert!(path.to_string_lossy().contains(".respondami"));
    assert!(path.to_string_lossy().contains("logs"));
}

#[test]
fn log_file_path_includes_respondami_dir_and_logs() {
    let path = log_file_path();
    assert!(path.to_string_lossy().contains(".respondami"));
    assert!(path.to_string_lossy().contains("logs"));
    assert_eq!(path.file_name().map(|n| n.to_string_lossy().to_string()), Some(LOG_FILE.to_string()));
}

#[test]
fn log_file_path_is_inside_log_dir() {
    let dir = log_dir_path();
    let file = log_file_path();
    assert!(file.starts_with(&dir));
}
