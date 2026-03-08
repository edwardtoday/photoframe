use photoframe_domain::{device_id_from_mac_suffix, token_hex_from_bytes};

#[test]
fn device_id_uses_last_four_mac_bytes() {
    let device_id = device_id_from_mac_suffix([0x12, 0x34, 0xab, 0xcd]);
    assert_eq!(device_id, "pf-1234abcd");
}

#[test]
fn token_bytes_are_lower_hex_encoded() {
    let token = token_hex_from_bytes([
        0x00, 0x11, 0x22, 0x33, 0xaa, 0xbb, 0xcc, 0xdd, 0xde, 0xad, 0xbe, 0xef, 0x10, 0x20, 0x30,
        0x40,
    ]);
    assert_eq!(token, "00112233aabbccdddeadbeef10203040");
}
