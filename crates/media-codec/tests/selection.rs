use media_codec::{
    BackendPreference, CodecCapability, EncoderBackend, VideoCodec, choose_h264_backend,
};

fn capability(backend: EncoderBackend, available: bool) -> CodecCapability {
    CodecCapability {
        codec: VideoCodec::H264,
        backend: backend.id(),
        hardware: backend.is_hardware(),
        available,
        reason: (!available).then(|| "native initialization failed".into()),
        dynamic_quality: false,
        dynamic_bitrate: true,
        codec_string: available.then(|| "avc1.42E01F".into()),
        rate_update: if backend.is_hardware() {
            "restart"
        } else {
            "live"
        },
    }
}

#[test]
fn auto_selects_a_validated_hardware_encoder_then_software() {
    let caps = [
        capability(EncoderBackend::OpenH264, true),
        capability(EncoderBackend::Nvenc, false),
        capability(EncoderBackend::Vaapi, true),
    ];
    assert_eq!(
        choose_h264_backend(BackendPreference::Auto, &caps, &[]),
        Some(EncoderBackend::Vaapi)
    );
    assert_eq!(
        choose_h264_backend(BackendPreference::Auto, &caps, &[EncoderBackend::Vaapi]),
        Some(EncoderBackend::OpenH264)
    );
    assert_eq!(
        choose_h264_backend(
            BackendPreference::Auto,
            &caps,
            &[EncoderBackend::Vaapi, EncoderBackend::OpenH264]
        ),
        None
    );
}

#[test]
fn manual_choice_is_respected_and_an_unusable_choice_falls_back_to_software() {
    let caps = [
        capability(EncoderBackend::OpenH264, true),
        capability(EncoderBackend::Nvenc, true),
        capability(EncoderBackend::Vaapi, true),
    ];
    assert_eq!(
        choose_h264_backend(BackendPreference::Software, &caps, &[]),
        Some(EncoderBackend::OpenH264)
    );
    assert_eq!(
        choose_h264_backend(BackendPreference::Vaapi, &caps, &[]),
        Some(EncoderBackend::Vaapi)
    );
    assert_eq!(
        choose_h264_backend(BackendPreference::Amf, &caps, &[]),
        Some(EncoderBackend::OpenH264)
    );
    assert_eq!(
        choose_h264_backend(BackendPreference::Vaapi, &caps, &[EncoderBackend::Vaapi]),
        Some(EncoderBackend::OpenH264)
    );
}

#[test]
fn partial_or_inconsistent_capability_metadata_is_not_a_success() {
    let software = capability(EncoderBackend::OpenH264, true);
    let mut hw = capability(EncoderBackend::Nvenc, true);
    hw.reason = Some("driver failed".into());
    assert_eq!(
        choose_h264_backend(
            BackendPreference::Auto,
            &[software.clone(), hw.clone()],
            &[]
        ),
        Some(EncoderBackend::OpenH264)
    );
    hw.reason = None;
    hw.codec_string = None;
    assert_eq!(
        choose_h264_backend(
            BackendPreference::Auto,
            &[software.clone(), hw.clone()],
            &[]
        ),
        Some(EncoderBackend::OpenH264)
    );
    hw.codec_string = Some("jpeg".into());
    assert_eq!(
        choose_h264_backend(BackendPreference::Auto, &[software, hw], &[]),
        Some(EncoderBackend::OpenH264)
    );
}
