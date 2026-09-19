//! VNN 真实图片集成测试
use lycore::vnn::{activate, extract_features};
use std::path::Path;

#[test]
#[ignore = "需要 ffmpeg + smoke 帧"]
fn terminal_frame_routes_to_terminal() {
    let frame = Path::new("../smoke/pack_final/frames/u001_f0.webp");
    if !frame.exists() {
        eprintln!("skip");
        return;
    }
    let feat = extract_features("ffmpeg", frame).unwrap();
    println!("edge={:.3} dark={:.3}", feat.edge_density, feat.dark_ratio);
    let act = activate(&feat, 3);
    println!("激活: {:?}", act);
    assert_eq!(act[0].0, "terminal");
}

#[test]
#[ignore = "需要 ffmpeg"]
fn identify_produces_experts() {
    let frame = Path::new("../smoke/pack_final/frames/u003_f1.webp");
    if !frame.exists() {
        eprintln!("skip");
        return;
    }
    let (verdict, conf, experts, queue) = lycore::vnn::identify("ffmpeg", frame).unwrap();
    println!("verdict={verdict} conf={conf:.2} queue={queue:?}");
    assert!(!verdict.is_empty());
    assert!(!experts.is_empty());
}
