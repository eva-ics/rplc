use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut builder = rplc::builder::Builder::new("plc.yml");
    builder.insert("modbus_server_port", &9503);
    builder.generate()?;
    //rplc::builder::generate("plc.yml")?;
    Ok(())
}
