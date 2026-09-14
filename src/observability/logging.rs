use flexi_logger::{Duplicate, FileSpec, Logger, WriteMode};
use std::sync::OnceLock;

/// เก็บ LoggerHandle ไว้เรียก flush() ตอนปิดโปรแกรม (กันหาง log ที่ยังค้างใน buffer หาย)
static LOGGER_HANDLE: OnceLock<flexi_logger::LoggerHandle> = OnceLock::new();

/// flush บรรทัด log ที่ยังค้างใน buffer ลงไฟล์ — เรียกก่อน quit_event_loop
pub fn flush_log() {
    if let Some(handle) = LOGGER_HANDLE.get() {
        handle.flush();
    }
}

/// ฟังก์ชันจัดรูปแบบ Log สำหรับ flexi_logger
fn log_format(
    w: &mut dyn std::io::Write,
    now: &mut flexi_logger::DeferredNow,
    record: &log::Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "[{}] {} [{}:{}] {}",
        now.format("%Y-%m-%d %H:%M:%S%.3f"),
        record.level(),
        record.file().unwrap_or("unknown"),
        record.line().unwrap_or(0),
        record.args()
    )
}

/// ตั้งค่า flexi_logger เพื่อบันทึก log ลงไฟล์ และแสดงผลใน terminal
pub fn setup_logger() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 2) Fallback: ถ้าโหลด .env มาแล้วก็ยังไม่มี key RUST_LOG อยู่ดี (เช่นไฟล์ถูกแก้จนไม่เหลือบรรทัดนี้)
    //    ให้ตั้ง default เป็น "info" กันไว้ ไม่ให้เงียบไปเลยเพราะไม่มีค่าอะไรตั้งไว้
    if std::env::var("RUST_LOG").is_err() {
        // SAFETY: เรียกก่อน spawn thread ใดๆ จึงไม่มี data race
        unsafe { std::env::set_var("RUST_LOG", "info") };
    }

    let handle = Logger::try_with_env_or_str("info")?
        .log_to_file(
            FileSpec::default()
                .directory("logs") // บันทึกไว้ในโฟลเดอร์ logs/
                .basename("app"), // ชื่อไฟล์ตั้งต้น เช่น app_yyyy-mm-dd_hh-mm-ss.log
        )
        .format(log_format) // ใช้จัดรูปแบบ log แบบกำหนดเอง
        .duplicate_to_stderr(Duplicate::All) // แสดงผลออกทาง terminal (stderr) ไปพร้อมๆ กัน
        .write_mode(WriteMode::BufferAndFlush) // ทำงานแบบมีบัฟเฟอร์ช่วยเพิ่มความเร็ว
        .start()?;

    // เก็บ handle ไว้สำหรับ flush_log() ตอนปิดโปรแกรม
    let _ = LOGGER_HANDLE.set(handle);

    Ok(())
}
