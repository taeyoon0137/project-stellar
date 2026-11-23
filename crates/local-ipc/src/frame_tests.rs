//! frame 형식 test (ADR-0124 결정 3).

use super::*;

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0x0123, n)
}

fn samples() -> Vec<Frame> {
    vec![
        Frame::BeginEdit {
            request: 1,
            document: sid(1),
            targets: vec![sid(10), sid(11)],
            externals: vec![("token".to_string(), sid(20))],
            description: "카드 정리".to_string(),
        },
        Frame::Propose {
            request: 2,
            base_revision: "g0".to_string(),
            document: sid(1),
            commands: vec![("rename-node".to_string(), vec![0, 1, 2, 3])],
        },
        Frame::Complete { request: 3 },
        Frame::Reply {
            request: 3,
            result: "ok".to_string(),
            message: String::new(),
        },
    ]
}

#[test]
fn frames_round_trip_through_a_stream() {
    let mut wire = Vec::new();
    for frame in samples() {
        write_frame(&mut wire, &frame).unwrap();
    }
    let mut reader = wire.as_slice();
    for frame in samples() {
        assert_eq!(read_frame(&mut reader).unwrap(), frame);
    }
    assert_eq!(
        read_frame(&mut reader),
        Err(FrameError::Closed),
        "경계에서 끝"
    );
    for frame in samples() {
        assert_eq!(decode(&encode(&frame).unwrap()).unwrap(), frame);
    }
}

#[test]
fn malformed_frames_fail_closed() {
    let good = encode(&samples()[0]).unwrap();
    let mut magic = good.clone();
    magic[0] = b'X';
    assert_eq!(decode(&magic), Err(FrameError::BadMagic));
    let mut version = good.clone();
    version[2] = 2;
    assert_eq!(decode(&version), Err(FrameError::UnsupportedVersion(2)));
    let mut kind = good.clone();
    kind[3] = 9;
    assert_eq!(decode(&kind), Err(FrameError::UnknownKind(9)));
    assert_eq!(decode(&good[..good.len() - 1]), Err(FrameError::Truncated));
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(matches!(decode(&trailing), Err(FrameError::Malformed(_))));
    // payload 안 길이가 payload를 넘으면 할당 전에 거부합니다.
    let mut count = encode(&Frame::Complete { request: 1 }).unwrap();
    count[3] = 1;
    count[12..16].copy_from_slice(&20u32.to_le_bytes());
    count.extend_from_slice(&[0; 16]);
    count.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(decode(&count), Err(FrameError::Malformed(_))));
    // 머리의 길이가 상한을 넘으면 payload를 읽지 않습니다.
    let mut huge = encode(&Frame::Complete { request: 1 }).unwrap();
    huge[12..16].copy_from_slice(&(MAX_FRAME_BYTES as u32).to_le_bytes());
    assert!(matches!(decode(&huge), Err(FrameError::TooLarge(_))));
    // UTF-8이 아닌 문자열.
    let mut text = encode(&Frame::Reply {
        request: 1,
        result: "ok".to_string(),
        message: "a".to_string(),
    })
    .unwrap();
    let last = text.len() - 1;
    text[last] = 0xff;
    assert!(matches!(decode(&text), Err(FrameError::Malformed(_))));
}

#[test]
fn oversized_frames_are_not_written() {
    let frame = Frame::Propose {
        request: 1,
        base_revision: "g0".to_string(),
        document: sid(1),
        commands: vec![("rename-node".to_string(), vec![0; MAX_FRAME_BYTES])],
    };
    assert!(matches!(encode(&frame), Err(FrameError::TooLarge(_))));
}
