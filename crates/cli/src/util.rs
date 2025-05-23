use typed_path::Utf8NativePathBuf;

// For argp::FromArgs
pub fn native_path(value: &str) -> Result<Utf8NativePathBuf, String> {
    Ok(Utf8NativePathBuf::from(value))
}
