fn main() -> std::io::Result<()>{
    if cfg!(target_os = "windows") {
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", "Mythic Launcher")
            .set("OriginalFilename", "mythic.exe")
            .compile()?;
    }
    Ok(())
}
