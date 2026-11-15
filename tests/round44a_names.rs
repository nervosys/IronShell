//! Round 44a: the Windows name refusal, as the dispatcher words it and as
//! `--explain` and the archive extractors apply it.

use iron_shell::safety::windows_param_name_refusal;

#[test]
fn a_write_is_told_its_write_would_land_elsewhere_not_its_read() {
    let w = windows_param_name_refusal("file_write", "w.txt.", true).expect("refused");
    assert!(w.message.contains("the write would land"), "{}", w.message);
    let r = windows_param_name_refusal("file_read", "w.txt.", false).expect("refused");
    assert!(r.message.contains("the read would land"), "{}", r.message);
}

#[test]
fn a_stream_name_is_left_to_the_write_guard() {
    // The dispatcher lets `name:stream` through for a read and a write alike;
    // `safety::guard` refuses it for a write.
    assert!(windows_param_name_refusal("file_write", "f.txt:s", true).is_none());
    assert!(windows_param_name_refusal("file_read", "f.txt:s", false).is_none());
}
