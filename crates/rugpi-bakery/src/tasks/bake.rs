//! Creates an image.

use std::fs;

use anyhow::anyhow;
use camino::Utf8Path;
use clap::Parser;
use rugpi_common::patch_cmdline;
use tempdir::TempDir;
use xscript::{read_str, run, Run};

use crate::utils::{LoopDevice, Mounted};

#[derive(Debug, Parser)]
pub struct BakeTask {
    /// The archive with the system files.
    archive: String,
    /// The output image.
    image: String,
}

pub fn run(task: &BakeTask) -> anyhow::Result<()> {
    let archive = Utf8Path::new(&task.archive);
    let image = Utf8Path::new(&task.image);
    let size = calculate_size(archive)?;
    println!("Size: {} bytes", size);
    fs::remove_file(image).ok();
    println!("Creating image...");
    run!(["fallocate", "-l", "{size}", image])?;
    run!(["sfdisk", image].with_stdin(IMAGE_LAYOUT))?;
    let disk_id = read_str!(["sfdisk", "--disk-id", image])?
        .strip_prefix("0x")
        .ok_or_else(|| anyhow!("`sfdisk` returned invalid disk id"))?
        .to_owned();
    let loop_device = LoopDevice::attach(image)?;
    println!("Creating file systems...");
    run!(["mkfs.vfat", "-n", "CONFIG", loop_device.partition(1)])?;
    run!(["mkfs.vfat", "-n", "BOOT-A", loop_device.partition(2)])?;
    run!(["mkfs.vfat", "-n", "BOOT-B", loop_device.partition(3)])?;
    run!(["mkfs.ext4", "-L", "system-a", loop_device.partition(5)])?;
    let temp_dir = TempDir::new("rugpi")?;
    let temp_dir_path = Utf8Path::from_path(temp_dir.path()).unwrap();
    {
        let _mounted_root = Mounted::mount(loop_device.partition(5), temp_dir_path)?;
        let boot_dir = temp_dir_path.join("boot");
        fs::create_dir_all(&boot_dir)?;
        let _mounted_boot = Mounted::mount(loop_device.partition(2), &boot_dir)?;
        println!("Writing system files...");
        run!(["tar", "-x", "-f", &task.archive, "-C", temp_dir_path])?;
        println!("Patching `cmdline.txt`...");
        patch_cmdline(
            boot_dir.join("cmdline.txt"),
            format!("PARTUUID={disk_id}-05"),
            //format!("UUID={root_uuid}"),
        )?;
        // println!("Patching `/etc/fstab`...");
        // patch_fstab(&temp_dir_path.join("etc/fstab"), &disk_id)?;
    }
    {
        let _mounted_config = Mounted::mount(loop_device.partition(1), temp_dir_path)?;
        run!(["cp", "-rTp", "/usr/share/rugpi/files/config", temp_dir_path])?;
        run!([
            "cp",
            "-f",
            "/usr/share/rugpi/rpi-eeprom/firmware/stable/pieeprom-2023-05-11.bin",
            temp_dir_path.join("pieeprom.upd")
        ])?;
        run!([
            "/usr/share/rugpi/rpi-eeprom/rpi-eeprom-digest",
            "-i",
            temp_dir_path.join("pieeprom.upd"),
            "-o",
            temp_dir_path.join("pieeprom.sig")
        ])?;
        run!([
            "cp",
            "-f",
            "/usr/share/rugpi/rpi-eeprom/firmware/stable/vl805-000138c0.bin",
            temp_dir_path.join("vl805.bin")
        ])?;
        run!([
            "/usr/share/rugpi/rpi-eeprom/rpi-eeprom-digest",
            "-i",
            temp_dir_path.join("vl805.bin"),
            "-o",
            temp_dir_path.join("vl805.sig")
        ])?;
        run!([
            "cp",
            "-f",
            "/usr/share/rugpi/rpi-eeprom/firmware/stable/recovery.bin",
            temp_dir_path.join("recovery.bin")
        ])?;
    }
    Ok(())
}

// pub fn patch_cmdline(path: &Utf8Path, disk_id: &str) -> anyhow::Result<()> {
//     let cmdline = fs::read_to_string(path)?;
//     let mut parts = cmdline
//         .split_ascii_whitespace()
//         .filter(|part| !part.starts_with("root=") && !part.starts_with("init=") &&
// *part != "quiet")         .map(str::to_owned)
//         .collect::<Vec<_>>();
//     parts.push(format!("root=PARTUUID={disk_id}-05"));
//     parts.push("init=/usr/bin/rugpi-ctrl".to_owned());
//     fs::write(path, parts.join(" "))?;
//     Ok(())
// }

// pub fn patch_fstab(path: &Utf8Path, disk_id: &str) -> anyhow::Result<()> {
//     let fstab = fs::read_to_string(path)?;
//     let lines = fstab
//         .lines()
//         .map(|line| {
//             if line.starts_with("PARTUUID=") {
//                 let (_, tail) = line.split_once("-").unwrap();
//                 format!("PARTUUID={disk_id}-{tail}")
//             } else {
//                 line.to_owned()
//             }
//         })
//         .collect::<Vec<_>>();
//     fs::write(path, lines.join("\n"))?;
//     Ok(())
// }

fn calculate_size(archive: &Utf8Path) -> anyhow::Result<u64> {
    let archive_bytes = fs::metadata(archive)?.len();
    let total_bytes = archive_bytes + (256 + 128 + 128) * 1024 * 1024;
    let total_blocks = (total_bytes / 4096) + 1;
    let actual_blocks = (1.2 * (total_blocks as f64)) as u64;
    Ok(actual_blocks * 4096)
}

const IMAGE_LAYOUT: &str = r#"
label: dos
unit: sectors
grain: 4M

config   : type=0c, size=256M
boot-a   : type=0c, size=128M
boot-b   : type=0c, size=128M

extended : type=05

system-a : type=83
"#;
