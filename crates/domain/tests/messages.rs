use domain::{ChannelSeq, MessageId};
use uuid::Uuid;

#[test]
fn message_id_only_accepts_uuid_v7() {
    for version in 0..=15 {
        let bits = (u128::from(version as u8) << 76) | (2_u128 << 62);
        assert_eq!(
            MessageId::from_client(Uuid::from_u128(bits)).is_ok(),
            version == 7
        );
        assert_eq!(
            MessageId::try_from(Uuid::from_u128(bits)),
            MessageId::from_client(Uuid::from_u128(bits))
        );
    }
    for variant in [0_u128, 3_u128 << 62, 7_u128 << 61] {
        assert!(MessageId::from_client(Uuid::from_u128((7_u128 << 76) | variant)).is_err());
        assert!(MessageId::try_from(Uuid::from_u128((7_u128 << 76) | variant)).is_err());
    }
    let id = Uuid::now_v7();
    assert_eq!(MessageId::from_client(id).unwrap().as_uuid(), id);
    assert_eq!(MessageId::try_from(id).unwrap().as_uuid(), id);
}

#[test]
fn message_sequences_are_positive_with_an_explicit_beginning_cursor() {
    for value in [i64::MIN, -1, 0] {
        assert!(ChannelSeq::new(value).is_err());
        assert!(ChannelSeq::try_from(value).is_err());
    }
    for value in [1, i64::MAX] {
        assert_eq!(ChannelSeq::new(value).unwrap().get(), value);
        assert_eq!(ChannelSeq::try_from(value).unwrap().get(), value);
    }
    assert_eq!(ChannelSeq::BEGINNING.get(), 0);
}
