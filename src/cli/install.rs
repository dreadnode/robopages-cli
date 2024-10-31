use std::io::Write;

use camino::Utf8PathBuf;

use super::InstallArgs;

pub(crate) async fn install(args: InstallArgs) -> anyhow::Result<()> {
    let path = Utf8PathBuf::from(
        shellexpand::full(args.path.as_str())
            .map_err(|e| anyhow::anyhow!("failed to expand path: {}", e))?
            .into_owned(),
    );
    if path.exists() {
        return Err(anyhow::anyhow!("{:?} already exists", path));
    }

    if args.source.ends_with(".zip") {
        // install from zip archive
        log::info!("extracting archive {} to {:?}", &args.source, &path);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&args.source)?)?;
        zip.extract(path)?;
    } else {
        // install from github repository
        let source = if !args.source.contains("://") {
            format!(
                "https://github.com/{}/archive/refs/heads/main.zip",
                &args.source
            )
        } else {
            format!("{}/archive/refs/heads/main.zip", &args.source)
        };

        log::info!("downloading robopages from {} ...", source);

        let temp_file = tempfile::NamedTempFile::new()?;
        let mut response = reqwest::get(&source).await?;
        let mut file = std::fs::File::create(temp_file.path())?;

        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk)?;
        }

        log::info!("extracting to {:?}", &path);

        let mut archive = zip::ZipArchive::new(std::fs::File::open(temp_file.path())?)?;
        archive.extract(&path)?;
    }

    Ok(())
}
