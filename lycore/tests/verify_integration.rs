//! 真实 tesseract 集成: 对冒烟帧跑完整级联
use lycore::verify::{verify, Ocr};
use std::path::Path;

#[test]
#[ignore = "需要本机 tesseract + smoke 帧"]
fn cascade_on_real_frame() {
    let frame = Path::new("../smoke/pack_final/frames/u003_f1.webp");
    if !frame.exists() {
        eprintln!("skip: 帧不存在");
        return;
    }
    let ocr = Ocr::new();
    // u003 strong = [cargo, run] — 画面是 "PS D:\demo\hello_world> cargo run"
    let expected = vec!["cargo".to_string(), "run".to_string()];
    let v = verify(&ocr, frame, &expected, "eng", 0.5).expect("ocr ran");
    assert!(v.pass, "帧上确有 cargo run, 应 VERIFY_PASS, got {:?}", v);
    assert_eq!(v.route, "ocr");
}

#[test]
#[ignore = "需要本机 tesseract + smoke 帧"]
fn cascade_failure_routes_to_queue() {
    let frame = Path::new("../smoke/pack_final/frames/u000_f0.webp");
    if !frame.exists() {
        eprintln!("skip: 帧不存在");
        return;
    }
    let ocr = Ocr::new();
    // u000 是空白终端 + "scene 1" 标题, 期望词完全无关 → 应降级
    let expected = vec!["防火墙".to_string(), "nginx".to_string()];
    let v = verify(&ocr, frame, &expected, "eng", 0.5).expect("ocr ran");
    assert!(!v.pass);
    assert_eq!(v.route, "learning_queue");
    assert!(v.vnn_hint.is_some());
}
