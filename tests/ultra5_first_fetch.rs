use helios::ultra5::{
    BootFlash, FirstFetch, PowerOnReset, FLASH_BASE, FLASH_SIZE, OBP_3_31_0_FIRST_WORD,
    POWER_ON_RESET_NPC, POWER_ON_RESET_PC,
};

#[test]
fn ultrasparc_ii_i_power_on_reset_enters_red_state_at_flash_vector() {
    let reset = PowerOnReset::ultra_sparc_ii();
    assert_eq!(reset.pc, 0xffff_ffff_f000_0020);
    assert_eq!(reset.npc, 0xffff_ffff_f000_0024);
    assert_eq!(reset.pc, POWER_ON_RESET_PC);
    assert_eq!(reset.npc, POWER_ON_RESET_NPC);
    assert!(reset.memory_model_tso);
    assert!(reset.red_state);
    assert!(reset.fpu_enabled);
    assert!(reset.address_mask_64);
    assert!(reset.privileged);
    assert!(!reset.interrupts_enabled);
    assert!(reset.alternate_globals);
    assert!(!reset.current_little_endian);
    assert!(!reset.trap_little_endian);
    assert!(!reset.interrupt_globals);
    assert!(!reset.mmu_globals);
}

#[test]
fn first_fetch_reads_the_supplied_bytes_at_the_physical_reset_vector() {
    let mut image = vec![0xff; FLASH_SIZE];
    image[0x20..0x24].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    let flash = BootFlash::new(image).unwrap();
    assert_eq!(
        flash.first_fetch(),
        Ok(FirstFetch {
            pc: 0xffff_ffff_f000_0020,
            npc: 0xffff_ffff_f000_0024,
            physical_address: 0x01ff_f000_0020,
            flash_offset: 0x20,
            instruction: 0x1234_5678,
        })
    );
    assert!(flash.read_word(0xffff_ffff_f000_0020).is_err());
}

#[test]
fn obp_revision_word_matches_the_vendor_reset_vector() {
    assert_eq!(OBP_3_31_0_FIRST_WORD, 0xc4d8_08a0);
}

#[test]
fn flash_rejects_wrong_image_size_and_addresses_outside_the_window() {
    assert!(BootFlash::new(vec![0; FLASH_SIZE - 4]).is_err());
    assert!(BootFlash::new(vec![0; FLASH_SIZE + 4]).is_err());

    let flash = BootFlash::new(vec![0; FLASH_SIZE]).unwrap();
    assert!(flash.read_word(FLASH_BASE - 4).is_err());
    assert!(flash.read_word(FLASH_BASE + 2).is_err());
    assert!(flash.read_word(FLASH_BASE + FLASH_SIZE as u64).is_err());
    assert_eq!(flash.read_word(FLASH_BASE + FLASH_SIZE as u64 - 4), Ok(0));
}

#[test]
#[ignore = "requires the extracted original PROM under local/firmware/ultra5"]
fn original_ultra5_obp_3_31_por_fetch_matches_local_image() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/local/firmware/ultra5/ultra5-obp331-post310-1mb.bin"
    );
    // The extractor already uses Python 3. Keep the core's default build dependency-free.
    let hash = std::process::Command::new("python3")
        .args([
            "-c",
            "import hashlib, sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())",
        ])
        .stdin(std::fs::File::open(path).expect("extract patch 106121-18 first"))
        .output()
        .expect("Python 3 is required for the local firmware check");
    assert!(hash.status.success());
    assert_eq!(
        std::str::from_utf8(&hash.stdout).unwrap().trim(),
        "fa3fafed624d19562af2ff488154a8bf3bf16b77aecf305cb077800398abe616"
    );
    let image = std::fs::read(path).unwrap();
    assert_eq!(image.len(), FLASH_SIZE);
    assert_eq!(&image[0..4], b"OBMD");
    assert_eq!(&image[16..20], b"OBP\0");
    assert_eq!(&image[0x80000..0x80004], b"OBMD");
    assert_eq!(&image[0x80010..0x80015], b"POST\0");

    let flash = BootFlash::new(image).unwrap();
    let fetch = flash.first_fetch().unwrap();
    assert_eq!(fetch.instruction, 0xc4d8_08a0);
    assert_eq!(fetch.pc, 0xffff_ffff_f000_0020);
    assert_eq!(fetch.npc, 0xffff_ffff_f000_0024);
    assert_eq!(fetch.physical_address, 0x01ff_f000_0020);
    assert_eq!(fetch.flash_offset, 0x20);
    println!(
        "POR pc={:#x} npc={:#x} physical={:#x} flash_offset={:#x} instruction={:#010x}",
        fetch.pc, fetch.npc, fetch.physical_address, fetch.flash_offset, fetch.instruction
    );
}
