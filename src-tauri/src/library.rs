//! Conceit 书库模型：索引结构、持久化与树操作。
//!
//! 书库目录位于 `<app_data_dir>/library`：
//! - `index.json`      书架索引（条目 + 每个容器内的排序）
//! - `manga/<id>/`     每部漫画导入时复制进来的图片

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const INDEX_VERSION: u32 = 1;
pub const INDEX_FILE_NAME: &str = "index.json";
pub const MANGA_DIR_NAME: &str = "manga";
/// 封面缩略图目录（相对书库根目录）
pub const THUMB_DIR_NAME: &str = "thumbs";
/// 缩略图长边上限（像素）；原图任一边超过它才生成缩略图
pub const THUMB_MAX_EDGE: u32 = 480;
/// 缩略图 JPEG 质量
pub const THUMB_QUALITY: u8 = 80;
/// 解码单张封面的内存预算（字节）。
///
/// 扫描漫画封面动辄 3000×4500（RGB 约 40MB），高清跨页能到 6000×9000
/// （162MB）甚至更大；`image::open` 不设上限，文件头写多大就分配多大。
/// 安卓单进程内存上限常见 256~512MB 且还要分给 WebView，一次这样的分配
/// 就会触发 lmkd 杀进程（或 Rust 分配失败 abort）——表现为导入时静默闪退。
/// 超过预算的封面放弃缩略图、书架回退原图（Chromium 给小尺寸 `<img>`
/// 做硬件解码 + 降采样，比在这里全幅解码安全得多）。
pub const THUMB_MAX_DECODE_BYTES: u64 = 96 * 1024 * 1024;

/// 支持的图片扩展名（小写判断）
pub const IMAGE_EXTENSIONS: [&str; 7] = ["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Manga,
    Folder,
}

/// 漫画专属元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaMeta {
    /// 相对于书库根目录的目录（统一使用 `/` 分隔）
    pub dir: String,
    /// 封面图片相对路径；无图片时为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// 封面缩略图路径（相对书库根目录，如 thumbs/m1.jpg）；
    /// None 表示未生成，前端回退显示原图封面
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// 全部图片相对路径（自然排序）
    pub pages: Vec<String>,
    /// 阅读进度（0.0 - 1.0）
    #[serde(default)]
    pub progress: f64,
}

/// 书架上的一个条目（漫画或文件夹）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub title: String,
    /// 所属文件夹 id；None 表示位于根目录
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// 是否展示封面图；false 时使用纯色封面（默认）
    #[serde(default)]
    pub show_cover: bool,
    /// kind == Manga 时存在
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manga: Option<MangaMeta>,
    /// 加入书库的 Unix 时间戳（秒）
    pub added_at: u64,
}

/// 书架索引
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryIndex {
    pub version: u32,
    pub items: Vec<Item>,
    /// 容器（"root" 或文件夹 id）-> 有序的子条目 id 列表
    pub order: BTreeMap<String, Vec<String>>,
    /// id 分配计数器
    pub next_seq: u64,
}

impl Default for LibraryIndex {
    fn default() -> Self {
        LibraryIndex {
            version: INDEX_VERSION,
            items: Vec::new(),
            order: BTreeMap::new(),
            next_seq: 1,
        }
    }
}

impl LibraryIndex {
    /// 容器键：None -> "root"
    pub fn container_key(parent: Option<&str>) -> String {
        parent.unwrap_or("root").to_string()
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(LibraryIndex::default());
        }
        let text = fs::read_to_string(path)
            .map_err(|e| format!("读取索引文件失败 {}: {e}", path.display()))?;
        if text.trim().is_empty() {
            return Ok(LibraryIndex::default());
        }
        serde_json::from_str(&text).map_err(|e| format!("解析索引文件失败: {e}"))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建索引目录失败 {}: {e}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| format!("序列化索引失败: {e}"))?;
        fs::write(path, text).map_err(|e| format!("写入索引文件失败: {e}"))
    }

    /// 分配一个新 id（m1 / f2 形式）
    pub fn next_id(&mut self, kind: ItemKind) -> String {
        let seq = self.next_seq;
        self.next_seq += 1;
        match kind {
            ItemKind::Manga => format!("m{seq}"),
            ItemKind::Folder => format!("f{seq}"),
        }
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    /// 按排序取某个容器的直接子条目
    pub fn children(&self, parent: Option<&str>) -> Vec<&Item> {
        let key = Self::container_key(parent);
        let ordered_ids = self.order.get(&key);
        let mut result: Vec<&Item> = Vec::new();
        if let Some(ids) = ordered_ids {
            for id in ids {
                if let Some(item) = self.item(id) {
                    // 防御：索引里的 parent 字段与容器键保持一致
                    if item.parent.as_deref() == parent {
                        result.push(item);
                    }
                }
            }
        }
        // 兜底：若有条目不在 order 中（理论上不应发生），按加入时间补在后面
        let known: std::collections::HashSet<&str> =
            result.iter().map(|i| i.id.as_str()).collect();
        for item in &self.items {
            if item.parent.as_deref() == parent && !known.contains(item.id.as_str()) {
                result.push(item);
            }
        }
        result
    }

    /// 递归统计每个文件夹内（含子文件夹）的漫画数量：文件夹 id -> 数量
    ///
    /// 先建父指针表，再逐部漫画沿父链向上累加，复杂度 O(n × 深度)。
    /// 放在数据层算一次，避免前端为每个文件夹卡片重复做线性查找。
    pub fn manga_counts(&self) -> BTreeMap<String, u32> {
        let parent_of: HashMap<&str, Option<&str>> = self
            .items
            .iter()
            .map(|i| (i.id.as_str(), i.parent.as_deref()))
            .collect();
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for item in &self.items {
            if item.kind != ItemKind::Manga {
                continue;
            }
            // 已访问集合：手工编辑索引可能造成环形父子关系，
            // 它保证每部漫画的父链最多被走一遍（正确性 + 终止性）
            let mut visited: HashSet<&str> = HashSet::new();
            let mut cur = item.parent.as_deref();
            while let Some(pid) = cur {
                if !visited.insert(pid) {
                    break;
                }
                *counts.entry(pid.to_string()).or_insert(0) += 1;
                cur = parent_of.get(pid).copied().flatten();
            }
        }
        counts
    }

    /// 把条目从原容器顺序中移除（不删除条目本身）
    pub fn detach(&mut self, id: &str) {
        for ids in self.order.values_mut() {
            ids.retain(|x| x != id);
        }
    }

    /// 追加到某个容器顺序末尾
    pub fn attach(&mut self, id: &str, parent: Option<&str>) {
        let key = Self::container_key(parent);
        let ids = self.order.entry(key).or_default();
        ids.retain(|x| x != id);
        ids.push(id.to_string());
    }

    /// 重新设置某个容器的顺序（校验 id 均属于该容器）
    pub fn set_order(&mut self, parent: Option<&str>, ordered_ids: Vec<String>) -> Result<(), String> {
        let valid: std::collections::HashSet<String> = self
            .children(parent)
            .into_iter()
            .map(|i| i.id.clone())
            .collect();
        let mut cleaned: Vec<String> = Vec::with_capacity(ordered_ids.len());
        for id in ordered_ids {
            if valid.contains(&id) && !cleaned.contains(&id) {
                cleaned.push(id);
            }
        }
        // 补充遗漏的（防御）
        for item in self.children(parent) {
            if !cleaned.contains(&item.id) {
                cleaned.push(item.id.clone());
            }
        }
        self.order
            .insert(Self::container_key(parent), cleaned);
        Ok(())
    }

    /// 删除条目；若是文件夹则递归删除其后代。返回所有被删除的条目。
    pub fn remove_recursive(&mut self, id: &str) -> Vec<Item> {
        let mut removed: Vec<Item> = Vec::new();
        let mut stack = vec![id.to_string()];
        while let Some(cur) = stack.pop() {
            if let Some(pos) = self.items.iter().position(|i| i.id == cur) {
                let item = self.items.remove(pos);
                // 子条目入栈
                for child in self.items.iter() {
                    if child.parent.as_deref() == Some(cur.as_str()) {
                        stack.push(child.id.clone());
                    }
                }
                self.detach(&cur);
                if let Some(ids) = self.order.remove(&cur) {
                    // 容器顺序随文件夹一起删除
                    let _ = ids;
                }
                removed.push(item);
            }
        }
        removed
    }

    /// 判断 target 是否是 id 的后代（含自身），用于防止把文件夹移进自己的子目录
    pub fn is_descendant_or_self(&self, id: &str, target: &str) -> bool {
        if id == target {
            return true;
        }
        let mut cur = Some(target.to_string());
        while let Some(c) = cur {
            if c == id {
                return true;
            }
            cur = self.item(&c).and_then(|i| i.parent.clone());
        }
        false
    }
}

/// 在内存预算内解码一张图片；超预算、格式不支持或解码器内部 panic 时返回 None。
///
/// 两个防护缺一不可：
/// - `Limits::max_allocation` 让 image 在**分配前**按文件头尺寸拒绝超大图
///   （见 image 0.25 `ImageReader::decode` 里的 `limits.reserve(total_bytes)`），
///   把「OOM 杀进程」降级成一次普通的解码失败；
/// - `catch_unwind` 兜住各解码器对畸形文件残留的 panic 路径——release 下
///   panic 会直接终止整个进程，而缩略图失败本该只是回退原图。
/// （前提是 Cargo.toml 不能用 `panic = "abort"`，否则 catch_unwind 不生效。）
fn decode_within_budget(path: &Path, budget: u64) -> Option<image::DynamicImage> {
    let decoded = std::panic::catch_unwind(|| {
        // image 默认只带 512MB 的总分配上限，远高于安卓单进程预算，
        // 这里按预算显式调低；超限时 decode() 在分配前就返回 Limits 错误
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(budget);
        let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
        reader.limits(limits);
        reader.decode().ok()
    });
    match decoded {
        Ok(img) => img,
        Err(_) => {
            eprintln!(
                "解码图片时解码器内部 panic（已忽略，封面将回退原图）: {}",
                path.display()
            );
            None
        }
    }
}

/// 为漫画封面生成缩略图，返回相对书库根目录的路径（如 `thumbs/m1.jpg`）。
///
/// 书架封面只有一百多像素宽，直接加载原图会迫使 WebView 解码整张大图
/// （扫描漫画常见 3000×4500、单文件数 MB）；这里在导入时预生成一份小图，
/// 阅读时仍加载原图，画质不受影响。
/// 原图本身不大于阈值、解码失败（含超出内存预算）时返回 None，调用方回退到原图封面。
/// 注意：输出为 JPEG，不带透明通道（漫画封面基本不涉及透明）。
pub fn make_cover_thumb(root: &Path, id: &str, cover_rel: &str) -> Option<String> {
    let src = root.join(MANGA_DIR_NAME).join(id).join(cover_rel);
    let img = decode_within_budget(&src, THUMB_MAX_DECODE_BYTES)?;
    let (w, h) = (img.width(), img.height());
    let longest = w.max(h);
    if longest <= THUMB_MAX_EDGE {
        return None; // 原图已经够小，没有必要
    }
    let scale = THUMB_MAX_EDGE as f32 / longest as f32;
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    let rgb = img.thumbnail(nw, nh).to_rgb8();
    let dir = root.join(THUMB_DIR_NAME);
    fs::create_dir_all(&dir).ok()?;
    let rel = format!("{THUMB_DIR_NAME}/{id}.jpg");
    let file = fs::File::create(root.join(&rel)).ok()?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(file, THUMB_QUALITY);
    enc.encode(rgb.as_raw(), nw, nh, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(rel)
}

/// 删除某部漫画的封面缩略图
pub fn remove_cover_thumb(root: &Path, id: &str) {
    let path = root.join(THUMB_DIR_NAME).join(format!("{id}.jpg"));
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

/// 把外部图片文件复制进某部漫画的目录，返回新文件在漫画目录内的相对路径。
///
/// 新图片一律平铺到漫画目录根部；与既有文件重名时自动加 " (n)" 序号，
/// 不覆盖任何已有页面。调用方负责把返回的名称追加进 `pages`。
pub fn copy_images_into_manga(
    root: &Path,
    id: &str,
    src_paths: &[PathBuf],
) -> Result<Vec<String>, String> {
    let dest_dir = root.join(MANGA_DIR_NAME).join(id);
    fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("创建漫画目录失败 {}: {e}", dest_dir.display()))?;
    let mut added: Vec<String> = Vec::new();
    for src in src_paths {
        if !src.is_file() {
            return Err(format!("不是文件: {}", src.display()));
        }
        if !is_image_file(src) {
            return Err(format!("不是图片文件: {}", src.display()));
        }
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("无法读取文件名: {}", src.display()))?;
        let rel = unique_file_name(&dest_dir, name);
        fs::copy(src, dest_dir.join(&rel))
            .map_err(|e| format!("复制图片失败 {}: {e}", src.display()))?;
        added.push(rel);
    }
    Ok(added)
}

/// 在 dir 下为 desired 生成不冲突的文件名（重名时在扩展名前加 " (n)"）
fn unique_file_name(dir: &Path, desired: &str) -> String {
    if !dir.join(desired).exists() {
        return desired.to_string();
    }
    let (stem, ext) = match desired.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, Some(e)),
        _ => (desired, None),
    };
    for n in 1..1000 {
        let candidate = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{stem} ({})", now_unix())
}

/// 删除某部漫画目录中的若干图片文件，返回实际删除的数量（文件不存在时跳过）。
/// 防御性拒绝绝对路径与 `..` 穿越——正常数据来自 scan_images，不会是这两种。
pub fn delete_manga_files(root: &Path, id: &str, pages: &[String]) -> usize {
    let dir = root.join(MANGA_DIR_NAME).join(id);
    let mut removed = 0;
    for rel in pages {
        let path = Path::new(rel);
        let escapes = path.is_absolute() || rel.split(['/', '\\']).any(|c| c == "..");
        if escapes {
            continue;
        }
        let full = dir.join(path);
        if full.is_file() && fs::remove_file(&full).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 判断两组页面是否同一集合（顺序不同也算一致）——自定义顺序前的校验，
/// 防止前端把增删伪装成重排传进来
pub fn same_page_set(current: &[String], next: &[String]) -> bool {
    if current.len() != next.len() {
        return false;
    }
    let mut a: Vec<&str> = current.iter().map(|s| s.as_str()).collect();
    let mut b: Vec<&str> = next.iter().map(|s| s.as_str()).collect();
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// 判断扩展名是否为图片
pub fn is_image_file(path: &Path) -> bool {    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// 自然排序比较（page2 < page10）
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let mut na = String::new();
                    while let Some(&c) = ai.peek() {
                        if c.is_ascii_digit() {
                            na.push(c);
                            ai.next();
                        } else {
                            break;
                        }
                    }
                    let mut nb = String::new();
                    while let Some(&c) = bi.peek() {
                        if c.is_ascii_digit() {
                            nb.push(c);
                            bi.next();
                        } else {
                            break;
                        }
                    }
                    let va: u64 = na.parse().unwrap_or(0);
                    let vb: u64 = nb.parse().unwrap_or(0);
                    match va.cmp(&vb) {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                } else {
                    let la = ca.to_lowercase().to_string();
                    let lb = cb.to_lowercase().to_string();
                    match la.cmp(&lb) {
                        Ordering::Equal => {
                            ai.next();
                            bi.next();
                        }
                        other => return other,
                    }
                }
            }
        }
    }
}

/// 递归收集目录下的图片，返回相对 base 的路径（`/` 分隔），按自然排序
pub fn scan_images(base: &Path) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    scan_images_inner(base, base, &mut found);
    found.sort_by(|a, b| natural_cmp(a, b));
    found
}

fn scan_images_inner(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut children: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    children.sort_by(|a, b| {
        natural_cmp(
            &a.file_name().unwrap_or_default().to_string_lossy(),
            &b.file_name().unwrap_or_default().to_string_lossy(),
        )
    });
    for path in children {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // 跳过隐藏文件与系统杂项
        if name.starts_with('.') || name == "__MACOSX" {
            continue;
        }
        if path.is_dir() {
            scan_images_inner(base, &path, out);
        } else if is_image_file(&path) {
            if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// 递归复制目录内容
pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建目录失败 {}: {e}", dst.display()))?;
    let entries =
        fs::read_dir(src).map_err(|e| format!("读取目录失败 {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("遍历目录失败: {e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "__MACOSX" {
            continue;
        }
        let target = dst.join(&name);
        if path.is_dir() {
            copy_dir_all(&path, &target)?;
        } else {
            fs::copy(&path, &target)
                .map_err(|e| format!("复制文件失败 {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// 从 zip 压缩包中解压出图片内容。
///
/// 整个解压包一层 `catch_unwind`：zip crate 对畸形压缩包的解析存在 panic
/// 路径，release 下 panic 会直接终止进程（表现为导入即闪退）。这里把它
/// 降级成一条普通错误，交由导入方报错并清理半成品目录。
pub fn extract_zip(src: &Path, dst: &Path) -> Result<(), String> {
    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    match std::panic::catch_unwind({
        let src = src.clone();
        move || extract_zip_inner(&src, &dst)
    }) {
        Ok(result) => result,
        Err(_) => Err(format!(
            "解压 zip 时内部 panic（压缩包可能已损坏）: {}",
            src.display()
        )),
    }
}

fn extract_zip_inner(src: &Path, dst: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| format!("打开 zip 失败 {}: {e}", src.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {e}"))?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        // 只保留图片，忽略目录与杂项
        if entry.is_dir() || !is_image_file(&enclosed) {
            continue;
        }
        let rel = enclosed
            .components()
            .filter(|c| !matches!(c, std::path::Component::ParentDir))
            .collect::<PathBuf>();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out_path = dst.join(rel);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建解压目录失败: {e}"))?;
        }
        let mut out = fs::File::create(&out_path)
            .map_err(|e| format!("创建解压文件失败 {}: {e}", out_path.display()))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("解压文件失败 {}: {e}", out_path.display()))?;
    }
    Ok(())
}

/// 从路径推断一个可读的标题
pub fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名".to_string())
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("conceit-test-{tag}-{}", now_unix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manga(id: &str, parent: Option<&str>) -> Item {
        Item {
            id: id.to_string(),
            kind: ItemKind::Manga,
            title: format!("漫画 {id}"),
            parent: parent.map(|s| s.to_string()),
            show_cover: false,
            manga: Some(MangaMeta {
                dir: format!("manga/{id}"),
                cover: Some("001.jpg".to_string()),
                thumb: None,
                pages: vec!["001.jpg".to_string()],
                progress: 0.0,
            }),
            added_at: 0,
        }
    }

    fn folder(id: &str, parent: Option<&str>) -> Item {
        Item {
            id: id.to_string(),
            kind: ItemKind::Folder,
            title: format!("文件夹 {id}"),
            parent: parent.map(|s| s.to_string()),
            show_cover: false,
            manga: None,
            added_at: 0,
        }
    }

    #[test]
    fn natural_sort_orders_numbers_correctly() {
        assert_eq!(natural_cmp("page2.jpg", "page10.jpg"), Ordering::Less);
        assert_eq!(natural_cmp("page10.jpg", "page2.jpg"), Ordering::Greater);
        assert_eq!(natural_cmp("a.jpg", "a.jpg"), Ordering::Equal);
        assert_eq!(natural_cmp("B.jpg", "a.jpg"), Ordering::Greater); // 大小写不敏感
        assert_eq!(natural_cmp("1-封面.jpg", "10-封面.jpg"), Ordering::Less);
    }

    #[test]
    fn scan_images_finds_nested_and_sorts_naturally() {
        let dir = temp_dir("scan");
        fs::write(dir.join("page10.jpg"), b"x").unwrap();
        fs::write(dir.join("page2.png"), b"x").unwrap();
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        fs::write(dir.join(".hidden.jpg"), b"x").unwrap();
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/page3.webp"), b"x").unwrap();
        fs::create_dir_all(dir.join("__MACOSX")).unwrap();
        fs::write(dir.join("__MACOSX/._junk.jpg"), b"x").unwrap();

        let pages = scan_images(&dir);
        assert_eq!(pages, vec!["page2.png", "page10.jpg", "sub/page3.webp"]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_dir_all_copies_nested_and_skips_junk() {
        let src = temp_dir("copy-src");
        let dst = temp_dir("copy-dst");
        fs::create_dir_all(src.join("a/b")).unwrap();
        fs::write(src.join("a/b/1.jpg"), b"1").unwrap();
        fs::write(src.join("2.png"), b"2").unwrap();
        fs::write(src.join(".DS_Store"), b"junk").unwrap();

        let target = dst.join("manga/m1");
        copy_dir_all(&src, &target).unwrap();

        assert!(target.join("a/b/1.jpg").exists());
        assert!(target.join("2.png").exists());
        assert!(!target.join(".DS_Store").exists());

        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn index_children_follow_order_and_fall_back() {
        let mut idx = LibraryIndex::default();
        idx.items.push(manga("m1", None));
        idx.items.push(manga("m2", None));
        idx.items.push(manga("m3", None));
        idx.attach("m1", None);
        idx.attach("m2", None);

        let ids: Vec<String> = idx.children(None).iter().map(|i| i.id.clone()).collect();
        assert_eq!(ids, vec!["m1", "m2", "m3"]); // m3 未排序，兜底追加

        idx.set_order(None, vec!["m3".into(), "m1".into()]).unwrap();
        let ids: Vec<String> = idx.children(None).iter().map(|i| i.id.clone()).collect();
        assert_eq!(ids, vec!["m3", "m1", "m2"]); // 遗漏的补在后面

        // 非法 id 被忽略
        idx.set_order(None, vec!["m1".into(), "不存在".into()]).unwrap();
        let ids: Vec<String> = idx.children(None).iter().map(|i| i.id.clone()).collect();
        assert_eq!(ids, vec!["m1", "m3", "m2"]);
    }

    #[test]
    fn manga_counts_recurse_and_skip_empty_folders() {
        let mut idx = LibraryIndex::default();
        // f1(空) / f2 -> m1, f3 -> m2 / m3, m4 在根目录
        idx.items.push(folder("f1", None));
        idx.items.push(folder("f2", None));
        idx.items.push(folder("f3", Some("f2")));
        idx.items.push(manga("m1", Some("f2")));
        idx.items.push(manga("m2", Some("f3")));
        idx.items.push(manga("m3", None));
        idx.items.push(manga("m4", None));

        let counts = idx.manga_counts();
        assert_eq!(counts.get("f1"), None); // 空文件夹不出现在表里
        assert_eq!(counts.get("f2"), Some(&2)); // 自身 1 部 + 子文件夹 1 部
        assert_eq!(counts.get("f3"), Some(&1));
        assert_eq!(counts.len(), 2); // 根目录的漫画不归属任何文件夹
    }

    #[test]
    fn manga_counts_survives_cyclic_parents() {
        // 手工编辑索引可能造出环形父子关系，统计必须能终止
        let mut idx = LibraryIndex::default();
        idx.items.push(manga("m1", Some("f1")));
        let mut f1 = folder("f1", Some("f2"));
        let mut f2 = folder("f2", Some("f1"));
        f1.kind = ItemKind::Folder;
        f2.kind = ItemKind::Folder;
        idx.items.push(f1);
        idx.items.push(f2);

        let counts = idx.manga_counts();
        assert_eq!(counts.get("f1"), Some(&1));
        assert_eq!(counts.get("f2"), Some(&1));
    }

    #[test]
    fn cover_thumb_downscales_large_image() {
        let root = temp_dir("thumb");
        let src = root.join(MANGA_DIR_NAME).join("m1");
        fs::create_dir_all(&src).unwrap();
        let img = image::RgbImage::from_pixel(600, 900, image::Rgb([200, 60, 60]));
        img.save(src.join("001.png")).unwrap();

        let rel = make_cover_thumb(&root, "m1", "001.png").expect("应生成缩略图");
        assert_eq!(rel, "thumbs/m1.jpg");
        let out = root.join(&rel);
        assert!(out.exists());
        // 等比缩放到长边 480
        let (w, h) = image::image_dimensions(&out).unwrap();
        assert_eq!((w, h), (320, 480));
        assert!(
            fs::metadata(&out).unwrap().len()
                < fs::metadata(src.join("001.png")).unwrap().len()
        );

        // 再生成一次应覆盖旧的（封面页变化时依赖这一点）
        assert_eq!(make_cover_thumb(&root, "m1", "001.png"), Some(rel));

        remove_cover_thumb(&root, "m1");
        assert!(!out.exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cover_thumb_skips_small_and_missing_images() {
        let root = temp_dir("thumb-small");
        let src = root.join(MANGA_DIR_NAME).join("m1");
        fs::create_dir_all(&src).unwrap();
        // 原图本身不大于阈值 → 不生成
        image::RgbImage::from_pixel(300, 200, image::Rgb([10, 10, 10]))
            .save(src.join("001.png"))
            .unwrap();
        assert!(make_cover_thumb(&root, "m1", "001.png").is_none());
        assert!(!root.join(THUMB_DIR_NAME).exists());

        // 文件不存在 / 不是图片 → 返回 None 而不 panic
        assert!(make_cover_thumb(&root, "m1", "不存在.png").is_none());
        fs::write(src.join("notes.txt"), b"x").unwrap();
        assert!(make_cover_thumb(&root, "m1", "notes.txt").is_none());

        // 删除不存在的缩略图不应报错
        remove_cover_thumb(&root, "m1");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn decode_within_budget_guards_memory_and_bad_input() {
        let root = temp_dir("decode-budget");
        let src = root.join(MANGA_DIR_NAME).join("m1");
        fs::create_dir_all(&src).unwrap();
        image::RgbImage::from_pixel(600, 900, image::Rgb([200, 60, 60]))
            .save(src.join("001.png"))
            .unwrap();

        // 预算充足 → 正常解出
        assert!(decode_within_budget(&src.join("001.png"), 64 * 1024 * 1024).is_some());
        // 预算不足（连 600x900 都装不下）→ 优雅返回 None，而不是 OOM 杀进程
        assert!(decode_within_budget(&src.join("001.png"), 1024).is_none());

        // 畸形 / 非图片文件：解码失败或被解码器内部 panic 都兜住，返回 None
        fs::write(src.join("broken.png"), b"\x89PNG\r\n\x1a\n not a real png").unwrap();
        assert!(decode_within_budget(&src.join("broken.png"), 64 * 1024 * 1024).is_none());
        fs::write(src.join("garbage.bin"), b"\x00\x01\x02\x03").unwrap();
        assert!(decode_within_budget(&src.join("garbage.bin"), 64 * 1024 * 1024).is_none());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn extract_zip_rejects_corrupt_archive_without_crashing() {
        let root = temp_dir("zip-broken");
        let src = root.join("broken.zip");
        // 只有 zip 魔数的残片：解析必须返回错误而不是 panic 杀进程
        fs::write(&src, b"PK\x03\x04 not really a zip").unwrap();
        assert!(extract_zip(&src, &root.join("out")).is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn remove_recursive_clears_folders_and_order() {
        let mut idx = LibraryIndex::default();
        idx.items.push(folder("f1", None));
        idx.items.push(manga("m1", Some("f1")));
        idx.items.push(folder("f2", Some("f1")));
        idx.items.push(manga("m2", Some("f2")));
        idx.items.push(manga("m3", None));
        idx.attach("f1", None);
        idx.attach("m1", Some("f1"));
        idx.attach("f2", Some("f1"));
        idx.attach("m2", Some("f2"));
        idx.attach("m3", None);

        let removed = idx.remove_recursive("f1");
        assert_eq!(removed.len(), 4);
        assert!(idx.item("f1").is_none());
        assert!(idx.item("m2").is_none());
        assert!(idx.item("m3").is_some());
        assert_eq!(idx.children(None).len(), 1);
        assert!(idx.order.get("f1").is_none());
    }

    #[test]
    fn descendant_check_prevents_cycles() {
        let mut idx = LibraryIndex::default();
        idx.items.push(folder("f1", None));
        idx.items.push(folder("f2", Some("f1")));
        idx.items.push(manga("m1", Some("f2")));

        assert!(idx.is_descendant_or_self("f1", "f1"));
        assert!(idx.is_descendant_or_self("f1", "f2"));
        assert!(idx.is_descendant_or_self("f1", "m1"));
        assert!(!idx.is_descendant_or_self("f2", "f1"));
        assert!(!idx.is_descendant_or_self("m1", "f1"));
    }

    #[test]
    fn title_from_path_uses_stem() {
        assert_eq!(title_from_path(&PathBuf::from("D:\\漫画\\海贼王")), "海贼王");
        assert_eq!(title_from_path(&PathBuf::from("/a/b/one piece.zip")), "one piece");
    }

    #[test]
    fn index_roundtrip_through_disk() {
        let dir = temp_dir("index");
        let path = dir.join("index.json");
        let mut idx = LibraryIndex::default();
        idx.items.push(manga("m1", None));
        idx.attach("m1", None);
        idx.save(&path).unwrap();

        let loaded = LibraryIndex::load(&path).unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.order.get("root").unwrap(), &vec!["m1".to_string()]);

        // 空文件 / 不存在都应得到默认索引
        fs::write(&path, "").unwrap();
        assert_eq!(LibraryIndex::load(&path).unwrap().items.len(), 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_images_into_manga_avoids_name_collisions() {
        let root = temp_dir("add-pages");
        let manga_dir = root.join(MANGA_DIR_NAME).join("m1");
        fs::create_dir_all(&manga_dir).unwrap();
        fs::write(manga_dir.join("001.jpg"), b"old").unwrap();

        let src = temp_dir("add-pages-src");
        fs::write(src.join("001.jpg"), b"new").unwrap(); // 与已有页重名
        fs::write(src.join("002.png"), b"new2").unwrap();
        fs::write(src.join("notes.txt"), b"junk").unwrap(); // 非图片应报错

        // 先只复制两张图片
        let added = copy_images_into_manga(
            &root,
            "m1",
            &[src.join("001.jpg"), src.join("002.png")],
        )
        .unwrap();
        assert_eq!(added, vec!["001 (1).jpg", "002.png"]);
        // 原文件不被覆盖
        assert_eq!(fs::read(manga_dir.join("001.jpg")).unwrap(), b"old");
        assert_eq!(fs::read(manga_dir.join("001 (1).jpg")).unwrap(), b"new");

        // 非图片整体报错，不产生半途结果
        let err = copy_images_into_manga(&root, "m1", &[src.join("notes.txt")]);
        assert!(err.is_err());

        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&src).ok();
    }

    #[test]
    fn delete_manga_files_skips_traversal_and_missing() {
        let root = temp_dir("del-pages");
        let manga_dir = root.join(MANGA_DIR_NAME).join("m1");
        fs::create_dir_all(manga_dir.join("sub")).unwrap();
        fs::write(manga_dir.join("001.jpg"), b"1").unwrap();
        fs::write(manga_dir.join("sub/002.jpg"), b"2").unwrap();
        // 目录外的文件，穿越删除必须被拒绝
        fs::write(root.join("secret.txt"), b"s").unwrap();

        let removed = delete_manga_files(
            &root,
            "m1",
            &[
                "001.jpg".to_string(),
                "sub/002.jpg".to_string(),
                "不存在.jpg".to_string(),
                "../secret.txt".to_string(),
            ],
        );
        assert_eq!(removed, 2);
        assert!(!manga_dir.join("001.jpg").exists());
        assert!(!manga_dir.join("sub/002.jpg").exists());
        assert!(root.join("secret.txt").exists());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn same_page_set_ignores_order_but_not_membership() {
        let a = vec!["1.jpg".to_string(), "2.jpg".to_string()];
        assert!(same_page_set(&a, &["2.jpg".into(), "1.jpg".into()]));
        // 多一页 / 少一页 / 换一页都不算同一集合
        assert!(!same_page_set(&a, &["1.jpg".into(), "2.jpg".into(), "3.jpg".into()]));
        assert!(!same_page_set(&a, &["1.jpg".into()]));
        assert!(!same_page_set(&a, &["1.jpg".into(), "3.jpg".into()]));
    }
}
