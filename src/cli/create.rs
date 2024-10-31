use super::CreateArgs;

pub(crate) async fn create(args: CreateArgs) -> anyhow::Result<()> {
    if args.name.exists() {
        return Err(anyhow::anyhow!("{:?} already exists", args.name));
    }

    for parts in args.template.get_data()? {
        if let Some(part_name) = parts.name {
            let asset = args.name.parent().unwrap().join(part_name);
            log::info!("creating asset {:?}", asset.to_string());

            std::fs::write(asset, parts.data)?;
        } else {
            log::info!(
                "creating {:?} from template {}",
                &args.name,
                args.template.to_string()
            );

            std::fs::write(&args.name, parts.data)?;
        }
    }

    Ok(())
}
