//! 最小只读 Apple SMC 客户端 —— 风扇与 die 温度（免 root）。
//!
//! 内核结构体三布局（字节偏移必须与 AppleSMC.kext 一致，勿动）：
//!   - **Jedda**（u32 dataSize、无显式 padding，80 字节）：lemon-cleaner
//!     CmcAppleSmc2.m（SMCReadKey2）的布局，其风扇转速读取在 Intel/Apple
//!     Silicon 上均实证有效；
//!   - **Classic**（data_size = IOByteCount = u64，88 字节）：lemon-cleaner
//!     CmcAppleSmc.m 的经典布局（部分 Intel 机型）；
//!   - **Modern**（u32 dataSize + 显式 padding，84 字节）：Stats 血统。
//! 三种布局在内核侧的字段偏移完全不同，传错布局所有键读取都会被内核拒绝。
//! 首次连接用 "#KEY" 逐个探测，命中即全局缓存（布局不随时间变化）。
//!
//! 其他对齐点：
//!   - 检查 `output.result != 0`（SMC 协议级错误码，lemon-cleaner 同款）；
//!   - 连接常驻复用（lemon-cleaner g_connect 同款），
//!     不再每次读取新开连接；
//!   - 读取 SMC 键不需要任何特权（只有*修改*风扇转速才需要特权 helper）。
//!
//! 传感器节流：SMC 读数变化慢，外层缓存 TTL 2.5s，
//! 避免气泡 1s/帧刷新时轰炸 IOKit。温度键按机型差异大，首次全键扫描
//! 分类后缓存键列表（CPU: Te*/TC*）。

use std::sync::Mutex;
use std::time::{Duration, Instant};

type KernReturn = i32;
const KERN_SUCCESS: KernReturn = 0;

// SMC 协议命令字（AppleSMC.kext 定义，勿改）
const SMC_CMD_READ_BYTES: u8 = 5;
const SMC_CMD_READ_INDEX: u8 = 8;
const SMC_CMD_READ_KEYINFO: u8 = 9;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    static kIOMainPortDefault: i32;
    fn IOServiceGetMatchingService(main_port: i32, matching: *const std::ffi::c_void) -> u32;
    fn IOServiceMatching(name: *const i8) -> *const std::ffi::c_void;
    fn IOServiceOpen(service: u32, task: u32, kind: u32, connect: *mut u32) -> KernReturn;
    fn IOObjectRelease(object: u32) -> KernReturn;
    fn IOConnectCallStructMethod(
        connection: u32,
        selector: u32,
        input: *const std::ffi::c_void,
        input_size: usize,
        output: *mut std::ffi::c_void,
        output_size: *mut usize,
    ) -> KernReturn;
    fn mach_task_self() -> u32;
}

// ── 内核结构：双布局（字节偏移必须与 AppleSMC.kext 一致，勿动） ──

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyDataVers {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyDataLimit {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    mem: u32,
}

/// Classic 布局 keyInfo：`IOByteCount` 在 64 位是 8 字节（Intel 内核期望）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyDataInfoClassic {
    data_size: u64,
    data_type: u32,
    data_attributes: u8,
}

/// Modern 布局 keyInfo：u32 dataSize（Stats 血统）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyDataInfoModern {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

/// Classic 布局（88 字节）：keyInfo 后无显式 padding，repr(C) 自然对齐
/// 得到 dataSize@32(u64) → result@48 → data8@50 → bytes@56，
/// 逐字节等价于 lemon-cleaner CmcAppleSmc.m 的 SMCParamStruct。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyDataClassic {
    key: u32,
    vers: KeyDataVers,
    limit: KeyDataLimit,
    info: KeyDataInfoClassic,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

/// Modern 布局（84 字节）：Stats 同款，keyInfo 与 result 之间的
/// `padding` 是 load-bearing 的——删掉它内核结构错位，读取返回 0/垃圾。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyDataModern {
    key: u32,
    vers: KeyDataVers,
    limit: KeyDataLimit,
    info: KeyDataInfoModern,
    padding: u16,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

/// Jedda 布局（80 字节）：逐字节等价于 lemon-cleaner CmcAppleSmc2.h 的
/// SMCKeyData_t（u32 dataSize、无显式 padding，repr(C) 下 keyInfo 的
/// 尾部对齐填充自然产生 28-39 占位）——其 SMCReadKey2 在 Intel 与
/// Apple Silicon 上均实证可读风扇转速。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyDataJedda {
    key: u32,
    vers: KeyDataVers,
    limit: KeyDataLimit,
    info: KeyDataInfoModern,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

/// 两种布局的统一访问接口（布局差异封装在此，上层逻辑不分叉）。
trait KeyDataBuf: Default + Sized {
    fn set_key(&mut self, key: u32);
    fn out_key(&self) -> u32;
    fn set_data8(&mut self, v: u8);
    fn set_data32(&mut self, v: u32);
    fn set_info_size(&mut self, size: u32);
    fn result(&self) -> u8;
    fn info_size(&self) -> u32;
    fn info_type(&self) -> u32;
    fn out_bytes(&self) -> &[u8];
}

impl KeyDataBuf for KeyDataClassic {
    fn set_key(&mut self, key: u32) {
        self.key = key;
    }
    fn out_key(&self) -> u32 {
        self.key
    }
    fn set_data8(&mut self, v: u8) {
        self.data8 = v;
    }
    fn set_data32(&mut self, v: u32) {
        self.data32 = v;
    }
    fn set_info_size(&mut self, size: u32) {
        self.info.data_size = size as u64;
    }
    fn result(&self) -> u8 {
        self.result
    }
    fn info_size(&self) -> u32 {
        self.info.data_size as u32
    }
    fn info_type(&self) -> u32 {
        self.info.data_type
    }
    fn out_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl KeyDataBuf for KeyDataModern {
    fn set_key(&mut self, key: u32) {
        self.key = key;
    }
    fn out_key(&self) -> u32 {
        self.key
    }
    fn set_data8(&mut self, v: u8) {
        self.data8 = v;
    }
    fn set_data32(&mut self, v: u32) {
        self.data32 = v;
    }
    fn set_info_size(&mut self, size: u32) {
        self.info.data_size = size;
    }
    fn result(&self) -> u8 {
        self.result
    }
    fn info_size(&self) -> u32 {
        self.info.data_size
    }
    fn info_type(&self) -> u32 {
        self.info.data_type
    }
    fn out_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl KeyDataBuf for KeyDataJedda {
    fn set_key(&mut self, key: u32) {
        self.key = key;
    }
    fn out_key(&self) -> u32 {
        self.key
    }
    fn set_data8(&mut self, v: u8) {
        self.data8 = v;
    }
    fn set_data32(&mut self, v: u32) {
        self.data32 = v;
    }
    fn set_info_size(&mut self, size: u32) {
        self.info.data_size = size;
    }
    fn result(&self) -> u8 {
        self.result
    }
    fn info_size(&self) -> u32 {
        self.info.data_size
    }
    fn info_type(&self) -> u32 {
        self.info.data_type
    }
    fn out_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn four_cc(s: &str) -> u32 {
    s.bytes().fold(0u32, |acc, b| (acc << 8) | b as u32)
}

fn type_str(t: u32) -> String {
    [
        (t >> 24 & 0xff) as u8,
        (t >> 16 & 0xff) as u8,
        (t >> 8 & 0xff) as u8,
        (t & 0xff) as u8,
    ]
    .iter()
    .map(|&b| b as char)
    .collect()
}

/// 布局选择：首次连接探测后全局缓存。
#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Jedda,
    Classic,
    Modern,
}

// 编译期布局哨兵：字节数必须精确等于 AppleSMC.kext 期望的结构体大小，
// 否则所有键读取都会被内核拒绝（Intel 实测教训）。
const _: () = assert!(std::mem::size_of::<KeyDataClassic>() == 88);
const _: () = assert!(std::mem::size_of::<KeyDataModern>() == 84);
const _: () = assert!(std::mem::size_of::<KeyDataJedda>() == 80);

struct Smc {
    conn: u32,
    available: bool,
    layout: Layout,
}

impl Smc {
    fn open() -> Smc {
        unsafe {
            let matching = IOServiceMatching(b"AppleSMC\0".as_ptr() as *const i8);
            let svc = IOServiceGetMatchingService(kIOMainPortDefault, matching);
            if svc == 0 {
                return Smc {
                    conn: 0,
                    available: false,
                    layout: Layout::Classic,
                };
            }
            let mut conn = 0u32;
            let rc = IOServiceOpen(svc, mach_task_self(), 0, &mut conn);
            IOObjectRelease(svc);
            if rc != KERN_SUCCESS {
                // SMC 连接失败，后续读数将返回 None
            }
            Smc {
                conn,
                available: rc == KERN_SUCCESS,
                layout: Layout::Jedda,
            }
        }
    }

    fn call<K: KeyDataBuf>(&self, input: &K, output: &mut K) -> KernReturn {
        let mut out_size = std::mem::size_of::<K>();
        unsafe {
            IOConnectCallStructMethod(
                self.conn,
                2,
                input as *const K as *const std::ffi::c_void,
                std::mem::size_of::<K>(),
                output as *mut K as *mut std::ffi::c_void,
                &mut out_size,
            )
        }
    }

    /// 读键的原始类型 + 字节（两段调用：key-info → read-bytes）。
    /// 对齐 lemon-cleaner：kern_return 与协议级 result 双重检查。
    fn read_raw_impl<K: KeyDataBuf>(&self, key: &str) -> Option<(String, Vec<u8>)> {
        if !self.available {
            return None;
        }
        let mut input = K::default();
        input.set_key(four_cc(key));
        input.set_data8(SMC_CMD_READ_KEYINFO);
        let mut output = K::default();
        if self.call(&input, &mut output) != KERN_SUCCESS || output.result() != 0 {
            return None;
        }
        let size = output.info_size() as usize;
        if size == 0 {
            return None;
        }
        let ty = type_str(output.info_type());
        let mut input2 = input;
        input2.set_info_size(output.info_size());
        input2.set_data8(SMC_CMD_READ_BYTES);
        let mut output2 = K::default();
        if self.call(&input2, &mut output2) != KERN_SUCCESS || output2.result() != 0 {
            return None;
        }
        let b = output2.out_bytes();
        let n = size.min(32);
        let mut arr = vec![0u8; size.max(4)];
        arr[..n].copy_from_slice(&b[..n]);
        Some((ty, arr))
    }

    /// 枚举索引处的四字符键名（仅全键扫描时用一次）。
    fn key_at_impl<K: KeyDataBuf>(&self, index: usize) -> Option<String> {
        if !self.available {
            return None;
        }
        let mut input = K::default();
        input.set_data8(SMC_CMD_READ_INDEX);
        input.set_data32(index as u32);
        let mut output = K::default();
        if self.call(&input, &mut output) != KERN_SUCCESS || output.result() != 0 {
            return None;
        }
        Some(type_str(output.out_key()))
    }

    fn read_raw(&self, key: &str) -> Option<(String, Vec<u8>)> {
        match self.layout {
            Layout::Jedda => self.read_raw_impl::<KeyDataJedda>(key),
            Layout::Classic => self.read_raw_impl::<KeyDataClassic>(key),
            Layout::Modern => self.read_raw_impl::<KeyDataModern>(key),
        }
    }

    fn key_at(&self, index: usize) -> Option<String> {
        match self.layout {
            Layout::Jedda => self.key_at_impl::<KeyDataJedda>(index),
            Layout::Classic => self.key_at_impl::<KeyDataClassic>(index),
            Layout::Modern => self.key_at_impl::<KeyDataModern>(index),
        }
    }

    /// 布局探测：读 "#KEY"（必存在），两段调用均成功才认定命中。
    fn probe_layout<K: KeyDataBuf>(&self, _name: &str) -> bool {
        let mut input = K::default();
        input.set_key(four_cc("#KEY"));
        input.set_data8(SMC_CMD_READ_KEYINFO);
        let mut output = K::default();
        let kr = self.call(&input, &mut output);
        if kr != KERN_SUCCESS {
            return false;
        }
        if output.result() != 0 {
            return false;
        }
        let size = output.info_size();
        let mut input2 = input;
        input2.set_info_size(size);
        input2.set_data8(SMC_CMD_READ_BYTES);
        let mut output2 = K::default();
        let kr2 = self.call(&input2, &mut output2);
        if kr2 != KERN_SUCCESS || output2.result() != 0 {
            return false;
        }
        true
    }

    /// 解码键值为 Double（覆盖全部 SMC 数值类型）。
    fn double(&self, key: &str) -> Option<f64> {
        let (ty, b) = self.read_raw(key)?;
        match ty.as_str() {
            "flt " => Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
            "ui8 " => Some(b[0] as f64),
            "ui16" => Some(u16::from_be_bytes([b[0], b[1]]) as f64),
            "ui32" => Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64),
            // fpe2：风扇 RPM 编码
            "fpe2" => Some((((b[0] as i32) << 6) + ((b[1] as i32) >> 2)) as f64),
            // sp78：有符号 7.8 定点——高字节必须符号扩展，否则零下温度读成 ~128-255°C
            "sp78" => Some((b[0] as i8 as i32 * 256 + b[1] as i32) as f64 / 256.0),
            _ => None,
        }
    }

    /// SMC 暴露的全部键名（数千个），慎用。
    fn all_keys(&self) -> Vec<String> {
        let Some(count) = self.double("#KEY") else {
            return Vec::new();
        };
        (0..count as usize).filter_map(|i| self.key_at(i)).collect()
    }
}

// ── 连接常驻 + 布局探测（lemon-cleaner g_connect 同款） ──

static SMC: Mutex<Option<Smc>> = Mutex::new(None);

/// 首次连接时探测布局：Jedda(80) → Classic(88) → Modern(84)，命中即锁定。
/// Jedda 优先：lemon-cleaner 的 SMCReadKey2 用它读风扇转速，在 Intel 与
/// Apple Silicon 上均实证有效；探测结果随连接全局缓存，只探测一次。
fn open_with_probe() -> Smc {
    let mut smc = Smc::open();
    if !smc.available {
        return smc;
    }
    if smc.probe_layout::<KeyDataJedda>("Jedda/80") {
        smc.layout = Layout::Jedda;
        return smc;
    }
    if smc.probe_layout::<KeyDataClassic>("Classic/88") {
        smc.layout = Layout::Classic;
        return smc;
    }
    if smc.probe_layout::<KeyDataModern>("Modern/84") {
        smc.layout = Layout::Modern;
        return smc;
    }
    eprintln!("[SMC] 布局探测：三种布局均失败，风扇/温度将无数据");
    smc
}

fn with_smc<R>(f: impl FnOnce(&Smc) -> R) -> Option<R> {
    let mut guard = SMC.lock().unwrap();
    if guard.is_none() {
        let smc = open_with_probe();
        if !smc.available {
            return None;
        }
        *guard = Some(smc);
    }
    guard.as_ref().map(f)
}

// ── 传感器层（TTL 缓存 + 温度键发现缓存） ──

/// 传感器 TTL：读数变化慢，2.5s 刷新一次足够。
const SENSOR_TTL: Duration = Duration::from_millis(2500);

struct SensorCache {
    at: Instant,
    fans: (i32, Vec<i32>),
    temps: Option<f64>,
}

struct TempKeys {
    cpu: Vec<String>,
}

static SENSOR_CACHE: Mutex<Option<SensorCache>> = Mutex::new(None);
/// 温度键发现结果跨 TTL 周期持久（键集合不随时间变化）。
static TEMP_KEYS: Mutex<Option<TempKeys>> = Mutex::new(None);

fn read_fans(smc: &Smc) -> (i32, Vec<i32>) {
    let Some(n) = smc.double("FNum") else {
        return (0, Vec::new());
    };
    let count = n as i32;
    if count <= 0 {
        return (0, Vec::new());
    }
    let rpm: Vec<i32> = (0..count)
        .map(|i| {
            let val = smc.double(&format!("F{}Ac", i)).unwrap_or(0.0).round() as i32;
            val
        })
        .collect();
    (count, rpm)
}

fn average(smc: &Smc, keys: &[String]) -> Option<f64> {
    let vals: Vec<f64> = keys
        .iter()
        .filter_map(|k| smc.double(k))
        .filter(|v| (10.0..=105.0).contains(v))
        .collect();
    if vals.is_empty() {
        return None;
    }
    Some(vals.iter().sum::<f64>() / vals.len() as f64)
}

/// 一次性全键扫描：按前缀分类 die 温度传感器（CPU: Te/TC；
/// 皮肤/电池/VRM/磁盘/GPU 传感器跳过）。
fn discover_temp_keys(smc: &Smc) -> TempKeys {
    let mut cpu = Vec::new();
    for key in smc.all_keys() {
        if !key.starts_with('T') {
            continue;
        }
        let Some(v) = smc.double(&key) else { continue };
        if !(10.0..=105.0).contains(&v) {
            continue;
        }
        if key.starts_with("Te") || key.starts_with("TC") {
            cpu.push(key);
        }
    }
    TempKeys { cpu }
}

fn read_temps(smc: &Smc) -> Option<f64> {
    // 键发现缓存跨 TTL 周期持久（只扫一次，之后按缓存键廉价复读）
    {
        let guard = TEMP_KEYS.lock().unwrap();
        if let Some(keys) = guard.as_ref() {
            return average(smc, &keys.cpu);
        }
    }
    let keys = discover_temp_keys(smc);
    let result = average(smc, &keys.cpu);
    *TEMP_KEYS.lock().unwrap() = Some(keys);
    result
}

fn read_sensors() -> ((i32, Vec<i32>), Option<f64>) {
    {
        let guard = SENSOR_CACHE.lock().unwrap();
        if let Some(c) = guard.as_ref() {
            if c.at.elapsed() < SENSOR_TTL {
                return (c.fans.clone(), c.temps);
            }
        }
    }
    let result = with_smc(|smc| (read_fans(smc), read_temps(smc)));
    let (fans, temps) = match result {
        Some(r) => r,
        None => ((0, Vec::new()), None),
    };
    *SENSOR_CACHE.lock().unwrap() = Some(SensorCache {
        at: Instant::now(),
        fans: fans.clone(),
        temps,
    });
    (fans, temps)
}

/// 风扇：(数量, 每个风扇的 RPM)。RPM 0 是有效值——凉快的 Mac 风扇停转。
/// 数量 0 = 本机读不到风扇（部分 Apple Silicon 机型正常），前端显示「无风扇数据」。
pub fn fans() -> (i32, Vec<i32>) {
    read_sensors().0
}

/// CPU die 温度 °C：CPU 簇均值。None = 无可读传感器。
/// 近似值 by design——SoC 有几十个传感器，不存在单一「CPU 温度」键。
pub fn temps() -> Option<f64> {
    read_sensors().1
}
