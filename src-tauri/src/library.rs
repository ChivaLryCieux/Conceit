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
        match serde_json::from_str(&text) {
            Ok(index) => Ok(index),
            Err(e) => {
                // 半截 JSON 不该让应用永远起不来：备份坏文件、降级为空索引
                // 启动。漫画文件都还在 manga/ 下，之后用书库体检的
                // 「孤儿目录收进书架」能逐部捞回，比卡在启动页好得多。
                // （save 已改为原子写入，正常情况下不会再产生半截文件，
                // 这里兜底磁盘损坏与更早版本写出的残留。）
                let backup = path.with_extension("corrupt.json");
                eprintln!(
                    "索引文件解析失败（已备份为 {}，降级为空索引启动）: {e}",
                    backup.display()
                );
                let _ = fs::rename(path, &backup);
                Ok(LibraryIndex::default())
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建索引目录失败 {}: {e}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| format!("序列化索引失败: {e}"))?;
        // 先写同目录临时文件再 rename：直接覆盖原文件时，进程若在写入中途
        // 被杀（安卓上 lmkd 随时可能）会留下半截 JSON，下次启动解析失败。
        // 同卷 rename 在 Windows / Unix 上都是原子替换，读者要么看到完整
        // 旧文件、要么看到完整新文件，不存在中间态。
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &text).map_err(|e| format!("写入索引文件失败 {}: {e}", tmp.display()))?;
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("写入索引文件失败 {}: {e}", path.display())
        })
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

    /// 登记一部空漫画（pages 为空、无封面）并挂到 parent 下，返回新条目。
    ///
    /// 只负责索引与排序：磁盘目录由调用方创建（见 commands::create_empty_manga），
    /// 之后用户用「添加图片」往里补图，封面自动落到第一页。
    pub fn insert_empty_manga(&mut self, title: &str, parent: Option<&str>) -> Item {
        let id = self.next_id(ItemKind::Manga);
        let item = Item {
            id: id.clone(),
            kind: ItemKind::Manga,
            title: title.to_string(),
            parent: parent.map(|s| s.to_string()),
            show_cover: false,
            manga: Some(MangaMeta {
                dir: format!("{MANGA_DIR_NAME}/{id}"),
                cover: None,
                thumb: None,
                pages: Vec::new(),
                progress: 0.0,
            }),
            added_at: now_unix(),
        };
        self.items.push(item.clone());
        self.attach(&id, parent);
        item
    }

    /// 把磁盘上已有的漫画目录收进书架——体检里「孤儿目录」的正向处理。
    ///
    /// 导入死在「复制文件」与「写索引」之间时，图片已经躺在 `manga/<id>/`
    /// 里，只是索引里没有条目、书架上看不见。这里按现有目录补一条索引，
    /// **不复制、不移动任何文件**：id 沿用目录名，封面取第一页并生成缩略图，
    /// 结果与正常导入一致。用户看不到应用内部目录，这是唯一能把它们捞回来的路。
    ///
    /// 目录里一张图片都没有时返回错误：空目录更可能是没复制完的残骸，
    /// 该让用户删掉，而不是变成一部点进去空白的漫画。
    pub fn adopt_manga_dir(
        &mut self,
        root: &Path,
        id: &str,
        title: &str,
        parent: Option<&str>,
    ) -> Result<Item, String> {
        if self.item(id).is_some() {
            return Err(format!("id {id} 已被占用"));
        }
        // 目录名即 id：若索引里已有条目指向同一目录，收进来就是两部漫画
        // 共享一份文件，删一部会连掉另一部的图
        if self
            .items
            .iter()
            .any(|i| i.kind == ItemKind::Manga && manga_dir_name(i) == id)
        {
            return Err(format!("目录 {MANGA_DIR_NAME}/{id} 已被索引里的条目占用"));
        }
        let dir = root.join(MANGA_DIR_NAME).join(id);
        let pages = scan_images(&dir);
        if pages.is_empty() {
            return Err(format!("目录 {MANGA_DIR_NAME}/{id} 里没有图片"));
        }
        let cover = pages.first().cloned();
        let thumb = cover.as_deref().and_then(|c| make_cover_thumb(root, id, c));
        let item = Item {
            id: id.to_string(),
            kind: ItemKind::Manga,
            title: title.to_string(),
            parent: parent.map(|s| s.to_string()),
            show_cover: false,
            manga: Some(MangaMeta {
                dir: format!("{MANGA_DIR_NAME}/{id}"),
                cover,
                thumb,
                pages,
                progress: 0.0,
            }),
            added_at: now_unix(),
        };
        self.items.push(item.clone());
        self.attach(id, parent);
        self.bump_seq_above(id);
        Ok(item)
    }

    /// 保证 id 分配计数器高于某个已占用的 id。
    ///
    /// 认领磁盘目录沿用目录名当 id（例如 m5），而 next_seq 可能还停在低位；
    /// 不抬上去的话，之后的导入会再分配到 m5，与刚收进书架的条目撞车，
    /// 把新文件合并进同一目录——正是体检要治的那个病。
    fn bump_seq_above(&mut self, id: &str) {
        if let Some(seq) = id
            .strip_prefix('m')
            .or_else(|| id.strip_prefix('f'))
            .and_then(|s| s.parse::<u64>().ok())
        {
            if self.next_seq <= seq {
                self.next_seq = seq + 1;
            }
        }
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

/// 体检报告里展示的文件名样本个数
const DIR_SAMPLE_LIMIT: usize = 8;

/// 漫画目录在磁盘上的实况（体检报告的「目录里实际有什么」）
#[derive(Debug, Clone, Serialize)]
pub struct DirStat {
    /// 目录名（即漫画 id，如 m3）
    pub id: String,
    /// 索引里的标题；索引中没有该目录时为 None（孤儿目录）
    pub title: Option<String>,
    /// 磁盘上文件总数（跳过隐藏文件与 `__MACOSX`，与复制口径一致）
    pub files: usize,
    /// 其中图片数（`scan_images` 口径，即书架实际能读到的页数）
    pub images: usize,
    /// 目录总字节
    pub bytes: u64,
    /// 前若干个文件名，让用户直接看到目录里装了什么
    pub sample: Vec<String>,
}

/// 索引里有条目、磁盘上却找不到对应目录
#[derive(Debug, Clone, Serialize)]
pub struct MissingEntry {
    pub id: String,
    pub title: String,
    /// 索引里记录的页数
    pub pages: usize,
}

/// 疑似重复的一组漫画（同名，且页面集合一致或页数相同）
#[derive(Debug, Clone, Serialize)]
pub struct DuplicateGroup {
    pub title: String,
    /// `pages_identical` = 页面集合完全相同，基本可确定是同一部漫画导了两次；
    /// `same_title_same_count` = 同名同页数但文件名不同（重复导入前后改过名）
    pub reason: &'static str,
    pub members: Vec<DuplicateMember>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateMember {
    pub id: String,
    /// 索引里记录的页数
    pub pages: usize,
    /// 磁盘上实际的图片数
    pub images: usize,
    /// 目录字节数
    pub bytes: u64,
    /// 加入书库的时间（Unix 秒）
    pub added_at: u64,
    /// 阅读进度 0-1（保留进度高的那份通常更合理）
    pub progress: f64,
    /// 该成员的目录是否已丢失
    pub dir_missing: bool,
}

/// 索引页数与磁盘图片数不一致
#[derive(Debug, Clone, Serialize)]
pub struct PageDrift {
    pub id: String,
    pub title: String,
    /// 索引里记录的页数
    pub indexed: usize,
    /// 磁盘上实际的图片数
    pub on_disk: usize,
}

/// 书库体检报告：把磁盘实况与索引对比后的全部差异。
///
/// 导入是「先复制文件、后写索引」两步，中间任何一步死掉都会留下对不上的
/// 状态：死在复制后会留下孤儿目录（书架上看不见、白占空间），死在写索引
/// 前则下一次导入会复用同一个 id 把内容合并进旧目录；而两次都成功落盘
/// 就是两部一模一样的漫画。这些后果平时无从察觉（用户看不到应用内部
/// 目录），体检把它们翻到明面上，并可一键清理。
#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    /// 索引中的漫画条目数
    pub indexed: usize,
    /// 磁盘 `manga/` 下的目录数
    pub on_disk: usize,
    /// 索引有条目、目录丢了
    pub missing_dirs: Vec<MissingEntry>,
    /// 目录在磁盘上、索引里没有
    pub orphan_dirs: Vec<DirStat>,
    /// 疑似重复
    pub duplicates: Vec<DuplicateGroup>,
    /// 索引页数与磁盘图片数不一致
    pub page_drift: Vec<PageDrift>,
    /// 磁盘上每个漫画目录的实况（含孤儿目录），按 id 排序
    pub disk_stats: Vec<DirStat>,
}

/// 漫画条目在磁盘上的目录名：取 `manga.dir` 的最后一段，与导入时
/// `manga/<id>` 的建目录规则一致；数据异常时退回 id 本身。
fn manga_dir_name(item: &Item) -> String {
    let Some(meta) = item.manga.as_ref() else {
        return item.id.clone();
    };
    meta.dir
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(item.id.as_str())
        .to_string()
}

fn manga_pages(item: &Item) -> &[String] {
    item.manga
        .as_ref()
        .map(|m| m.pages.as_slice())
        .unwrap_or(&[])
}

/// 递归遍历目录下的文件（跳过隐藏文件与 `__MACOSX`）
fn walk_files(dir: &Path, out: &mut dyn FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    for path in children {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.starts_with('.') || name == "__MACOSX" {
            continue;
        }
        if path.is_dir() {
            walk_files(&path, out);
        } else {
            out(&path);
        }
    }
}

/// 遍历 `manga/` 下每个目录，统计文件数 / 图片数 / 字节数并取文件名样本。
/// 一次遍历同时算出图片数（与 [`scan_images`] 同一套跳过规则），不再各走一遍。
fn scan_manga_dirs(manga_root: &Path) -> Vec<DirStat> {
    let mut out: Vec<DirStat> = Vec::new();
    let Ok(entries) = fs::read_dir(manga_root) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let id = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut files = 0usize;
        let mut images = 0usize;
        let mut bytes = 0u64;
        let mut names: Vec<String> = Vec::new();
        walk_files(&dir, &mut |path| {
            files += 1;
            if is_image_file(path) {
                images += 1;
            }
            if let Ok(meta) = fs::metadata(path) {
                bytes += meta.len();
            }
            if let Some(name) = path.file_name() {
                names.push(name.to_string_lossy().to_string());
            }
        });
        names.sort();
        names.truncate(DIR_SAMPLE_LIMIT);
        out.push(DirStat {
            id,
            title: None,
            files,
            images,
            bytes,
            sample: names,
        });
    }
    out
}

/// 体检：对比磁盘实况与索引，产出 [`HealthReport`]
pub fn build_health_report(root: &Path, index: &LibraryIndex) -> HealthReport {
    let manga_root = root.join(MANGA_DIR_NAME);
    let mut disk_stats = scan_manga_dirs(&manga_root);

    let mangas: Vec<&Item> = index
        .items
        .iter()
        .filter(|i| i.kind == ItemKind::Manga)
        .collect();
    // 磁盘实况补上索引里的标题，孤儿目录保持 None；
    // 目录名 -> 条目先建索引，避免每个目录都全量扫一遍漫画条目
    let item_by_dir: HashMap<String, &Item> =
        mangas.iter().map(|m| (manga_dir_name(m), *m)).collect();
    for stat in &mut disk_stats {
        stat.title = item_by_dir.get(&stat.id).map(|m| m.title.clone());
    }
    let stat_by_id: HashMap<&str, &DirStat> =
        disk_stats.iter().map(|s| (s.id.as_str(), s)).collect();

    // 1. 索引有条目、磁盘没有目录：写索引前就死了，或文件被外部清掉
    let mut missing_dirs = Vec::new();
    for item in &mangas {
        if !stat_by_id.contains_key(manga_dir_name(item).as_str()) {
            missing_dirs.push(MissingEntry {
                id: item.id.clone(),
                title: item.title.clone(),
                pages: manga_pages(item).len(),
            });
        }
    }

    // 2. 目录在、索引没有：复制完文件后死在写索引前，书架上看不见但白占空间
    let referenced: HashSet<String> = mangas.iter().map(|m| manga_dir_name(m)).collect();
    let orphan_dirs = disk_stats
        .iter()
        .filter(|s| !referenced.contains(&s.id))
        .cloned()
        .collect();

    // 3. 疑似重复：同名条目分组，页面集合一致（或同名同页数）才报，
    //    避免把用户故意建的两部同名漫画误判成重复
    let mut by_title: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for item in &mangas {
        by_title.entry(item.title.as_str()).or_default().push(item);
    }
    let mut duplicates = Vec::new();
    for (title, members) in by_title {
        if members.len() < 2 {
            continue;
        }
        let identical = members
            .iter()
            .all(|m| same_page_set(manga_pages(members[0]), manga_pages(m)));
        let same_count = members
            .iter()
            .map(|m| manga_pages(m).len())
            .collect::<HashSet<_>>()
            .len()
            == 1;
        if !identical && !same_count {
            continue;
        }
        duplicates.push(DuplicateGroup {
            title: title.to_string(),
            reason: if identical {
                "pages_identical"
            } else {
                "same_title_same_count"
            },
            members: members
                .iter()
                .map(|m| {
                    let stat = stat_by_id.get(manga_dir_name(m).as_str()).copied();
                    DuplicateMember {
                        id: m.id.clone(),
                        pages: manga_pages(m).len(),
                        images: stat.map(|s| s.images).unwrap_or(0),
                        bytes: stat.map(|s| s.bytes).unwrap_or(0),
                        added_at: m.added_at,
                        progress: m.manga.as_ref().map(|x| x.progress).unwrap_or(0.0),
                        dir_missing: stat.is_none(),
                    }
                })
                .collect(),
        });
    }

    // 4. 索引页数与磁盘图片数不一致（目录还在，但内容对不上）
    let mut page_drift = Vec::new();
    for item in &mangas {
        let Some(stat) = stat_by_id.get(manga_dir_name(item).as_str()).copied() else {
            continue;
        };
        let indexed = manga_pages(item).len();
        if indexed != stat.images {
            page_drift.push(PageDrift {
                id: item.id.clone(),
                title: item.title.clone(),
                indexed,
                on_disk: stat.images,
            });
        }
    }

    HealthReport {
        indexed: mangas.len(),
        on_disk: disk_stats.len(),
        missing_dirs,
        orphan_dirs,
        duplicates,
        page_drift,
        disk_stats,
    }
}

/// 重新扫描漫画目录，把索引里的页数 / 封面 / 缩略图修正为磁盘实况。
/// 返回是否有变化（没有变化时调用方不必落盘）。
///
/// 用于体检里的「索引页数与磁盘不一致」：文件被外部增删、或导入中途
/// 死去又复用同一个 id 把内容合并进旧目录，都会让索引和磁盘对不上。
pub fn resync_manga_from_disk(root: &Path, item: &mut Item) -> bool {
    let Some(meta) = item.manga.as_mut() else {
        return false;
    };
    let dir = root.join(MANGA_DIR_NAME).join(&item.id);
    let pages = scan_images(&dir);
    // 封面页还在就保留，否则顺延到第一页；一张不剩就置空
    let next_cover = match meta.cover.as_ref() {
        Some(c) if pages.iter().any(|p| p == c) => meta.cover.clone(),
        _ => pages.first().cloned(),
    };
    let cover_changed = next_cover != meta.cover;
    let changed = cover_changed || pages != meta.pages;
    meta.pages = pages;
    if cover_changed {
        meta.cover = next_cover.clone();
        // 封面换了缩略图要跟着重做（解码原图，只在真变了时才做）；
        // 没有封面则清掉残留在 thumbs/ 里的旧图
        meta.thumb = match next_cover.as_deref() {
            Some(c) => make_cover_thumb(root, &item.id, c),
            None => {
                remove_cover_thumb(root, &item.id);
                None
            }
        };
    }
    changed
}

/// 判断扩展名是否为图片
pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// 自然排序比较（page2 < page10）。
///
/// 这是最热的比较函数（导入扫盘 / 体检对每个文件名排序），数字段与
/// ASCII 字符都走零分配路径；只有非 ASCII 字符才退回 Unicode 小写展开。
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
                    // 边扫边累加数值；超长数字串饱和后视为相等
                    // （原先 parse 失败置 0，同为「视作相等」的兜底）
                    let mut va: u64 = 0;
                    while let Some(&c) = ai.peek() {
                        match c.to_digit(10) {
                            Some(d) => {
                                va = va.saturating_mul(10).saturating_add(u64::from(d));
                                ai.next();
                            }
                            None => break,
                        }
                    }
                    let mut vb: u64 = 0;
                    while let Some(&c) = bi.peek() {
                        match c.to_digit(10) {
                            Some(d) => {
                                vb = vb.saturating_mul(10).saturating_add(u64::from(d));
                                bi.next();
                            }
                            None => break,
                        }
                    }
                    match va.cmp(&vb) {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                } else {
                    let ord = if ca.is_ascii() && cb.is_ascii() {
                        ca.to_ascii_lowercase().cmp(&cb.to_ascii_lowercase())
                    } else {
                        let la = ca.to_lowercase().to_string();
                        let lb = cb.to_lowercase().to_string();
                        la.cmp(&lb)
                    };
                    match ord {
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

/// 复制 / 解压进度回调：每复制完一个文件回调一次该文件的字节数。
/// 传 `&mut |_| {}` 即忽略进度（测试与一次性内部调用用）。
pub type ProgressFn<'a> = &'a mut dyn FnMut(u64);

/// 统计目录树全部文件大小（进度条的分母）；与 [`copy_dir_all`] 的跳过规则
/// 保持一致。目录不存在等异常时该项计 0，调用方据此走不确定态。
pub fn dir_total_bytes(src: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "__MACOSX" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = fs::metadata(&path) {
                total += meta.len();
            }
        }
    }
    total
}

/// 递归复制目录内容；每复制完一个文件向 `on_progress` 回报字节数
pub fn copy_dir_all(src: &Path, dst: &Path, on_progress: ProgressFn<'_>) -> Result<(), String> {
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
            copy_dir_all(&path, &target, on_progress)?;
        } else {
            let copied = fs::copy(&path, &target)
                .map_err(|e| format!("复制文件失败 {}: {e}", path.display()))?;
            on_progress(copied);
        }
    }
    Ok(())
}

/// 从 zip 压缩包中解压出图片内容；每解压完一个条目向 `on_progress` 回报字节数。
///
/// 整个解压包一层 `catch_unwind`：zip crate 对畸形压缩包的解析存在 panic
/// 路径，release 下 panic 会直接终止进程（表现为导入即闪退）。这里把它
/// 降级成一条普通错误，交由导入方报错并清理半成品目录。
pub fn extract_zip(src: &Path, dst: &Path, on_progress: ProgressFn<'_>) -> Result<(), String> {
    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    let src_for_msg = src.clone();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe({
        let src = src.clone();
        move || {
            let mut on_progress = on_progress;
            extract_zip_inner(&src, &dst, &mut on_progress)
        }
    })) {
        Ok(result) => result,
        Err(_) => Err(format!(
            "解压 zip 时内部 panic（压缩包可能已损坏）: {}",
            src_for_msg.display()
        )),
    }
}

fn extract_zip_inner(src: &Path, dst: &Path, on_progress: &mut dyn FnMut(u64)) -> Result<(), String> {
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
        let copied = std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("解压文件失败 {}: {e}", out_path.display()))?;
        on_progress(copied);
    }
    Ok(())
}

/// zip 中实际会被解压的图片总字节数（进度条分母）；解析失败返回 0。
/// 只读中央目录，不解压数据，开销很小。
pub fn zip_total_bytes(src: &Path) -> u64 {
    let Ok(file) = fs::File::open(src) else {
        return 0;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return 0;
    };
    let mut total = 0u64;
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else {
            continue;
        };
        let will_extract = entry
            .enclosed_name()
            .is_some_and(|name| !entry.is_dir() && is_image_file(&name));
        if will_extract {
            total += entry.size();
        }
    }
    total
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
        let mut copied = 0u64;
        copy_dir_all(&src, &target, &mut |n| copied += n).unwrap();
        // 只统计真实复制的两个文件（.DS_Store 被跳过）
        assert_eq!(copied, 2);

        assert!(target.join("a/b/1.jpg").exists());
        assert!(target.join("2.png").exists());
        assert!(!target.join(".DS_Store").exists());

        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn dir_total_bytes_matches_what_copy_dir_all_copies() {
        let src = temp_dir("total-src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/a.jpg"), b"12345").unwrap();
        fs::write(src.join("b.png"), b"123").unwrap();
        fs::write(src.join(".DS_Store"), b"ignored").unwrap();

        // 分母必须和实际搬运的字节数一致，否则进度条会在中途冲到 100%
        assert_eq!(dir_total_bytes(&src), 8);

        let dst = temp_dir("total-dst");
        let mut copied = 0u64;
        copy_dir_all(&src, &dst.join("out"), &mut |n| copied += n).unwrap();
        assert_eq!(copied, dir_total_bytes(&src));

        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn dir_total_bytes_of_missing_dir_is_zero() {
        // 不存在时返回 0（调用方据此走不确定态），不能 panic
        assert_eq!(dir_total_bytes(Path::new("definitely-not-here-404")), 0);
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
        let mut ignored = 0u64;
        assert!(extract_zip(&src, &root.join("out"), &mut |n| ignored += n).is_err());
        assert_eq!(ignored, 0);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn zip_total_bytes_counts_only_images() {
        use std::io::Write;
        let root = temp_dir("zip-total");
        let src = root.join("book.zip");
        // 手写一个最小 zip：一个图片条目（内容 5 字节）+ 一个 txt（应被忽略）
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            w.start_file("p1.jpg", opts).unwrap();
            w.write_all(b"12345").unwrap();
            w.start_file("notes.txt", opts).unwrap();
            w.write_all(b"skipme").unwrap();
            w.finish().unwrap();
        }
        fs::write(&src, &buf).unwrap();

        assert_eq!(zip_total_bytes(&src), 5);

        // 非 zip / 不存在时返回 0，不能让导入崩在统计分母这一步
        fs::write(root.join("bad.zip"), b"PK\x03\x04 nope").unwrap();
        assert_eq!(zip_total_bytes(&root.join("bad.zip")), 0);
        assert_eq!(zip_total_bytes(&root.join("missing.zip")), 0);

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

        // 再次保存会原子替换旧文件，加载到的是新内容
        idx.items.push(manga("m2", None));
        idx.attach("m2", None);
        idx.save(&path).unwrap();
        assert_eq!(LibraryIndex::load(&path).unwrap().items.len(), 2);

        // 保存后不残留临时文件
        assert!(!dir.join("index.json.tmp").exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_recovers_from_corrupt_index_file() {
        let dir = temp_dir("index-corrupt");
        let path = dir.join("index.json");
        fs::write(&path, "{ 这不是合法 json").unwrap();

        // 半截 / 损坏的索引：备份坏文件后降级为空索引，而不是让应用起不来
        let idx = LibraryIndex::load(&path).unwrap();
        assert!(idx.items.is_empty());
        assert!(!path.exists());
        let backup = dir.join("index.corrupt.json");
        assert!(backup.exists());

        // 降级后可以正常继续保存 / 加载
        let mut idx = LibraryIndex::default();
        idx.items.push(manga("m1", None));
        idx.attach("m1", None);
        idx.save(&path).unwrap();
        assert_eq!(LibraryIndex::load(&path).unwrap().items.len(), 1);

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

    #[test]
    fn insert_empty_manga_registers_pageless_item() {
        let mut idx = LibraryIndex::default();
        idx.items.push(folder("f1", None));
        idx.attach("f1", None);

        let at_root = idx.insert_empty_manga("空漫画", None);
        let in_folder = idx.insert_empty_manga("另一部", Some("f1"));

        for it in [&at_root, &in_folder] {
            assert_eq!(it.kind, ItemKind::Manga);
            assert!(!it.show_cover); // 与导入一致：默认纯色封面
            let meta = it.manga.as_ref().expect("漫画必须带 meta");
            // 空漫画：没有图片、没有封面，目录按 id 生成
            assert!(meta.pages.is_empty());
            assert!(meta.cover.is_none());
            assert!(meta.thumb.is_none());
            assert_eq!(meta.progress, 0.0);
            assert_eq!(meta.dir, format!("manga/{}", it.id));
        }
        // 父子关系与容器顺序都登记到位
        assert_eq!(at_root.parent, None);
        assert_eq!(in_folder.parent.as_deref(), Some("f1"));
        assert_eq!(idx.children(None).len(), 2); // f1 + 空漫画
        assert_eq!(idx.children(Some("f1")).len(), 1);
        // id 互不重复（都走 m 前缀分配）
        assert_ne!(at_root.id, in_folder.id);
        assert!(at_root.id.starts_with('m'));
    }

    /// 指定标题与页数的漫画条目（体检测试用）
    fn manga_titled(id: &str, title: &str, pages: &[&str]) -> Item {
        let mut item = manga(id, None);
        item.title = title.to_string();
        item.manga = item.manga.map(|mut m| {
            m.pages = pages.iter().map(|s| s.to_string()).collect();
            m
        });
        item
    }

    #[test]
    fn health_report_flags_missing_orphan_duplicate_and_drift() {
        let root = temp_dir("health");
        let manga_root = root.join(MANGA_DIR_NAME);
        // m1：索引与磁盘一致
        fs::create_dir_all(manga_root.join("m1")).unwrap();
        fs::write(manga_root.join("m1/001.jpg"), b"12345").unwrap();
        fs::write(manga_root.join("m1/002.jpg"), b"123").unwrap();
        // m2：索引里有条目，磁盘上没有目录（写索引前就死了）
        // m3：磁盘上有目录，索引里没有（复制完死在写索引前 → 孤儿）
        fs::create_dir_all(manga_root.join("m3")).unwrap();
        fs::write(manga_root.join("m3/a.png"), b"1").unwrap();
        fs::write(manga_root.join("m3/b.png"), b"22").unwrap();
        fs::write(manga_root.join("m3/notes.txt"), b"xyz").unwrap();
        // m4 / m5：同名同页面 → 同一部漫画导了两次
        for id in ["m4", "m5"] {
            fs::create_dir_all(manga_root.join(id)).unwrap();
            fs::write(manga_root.join(format!("{id}/001.jpg")), b"12345").unwrap();
            fs::write(manga_root.join(format!("{id}/002.jpg")), b"123").unwrap();
        }
        // m6：索引记 2 页，磁盘只剩 1 张（页数漂移）
        fs::create_dir_all(manga_root.join("m6")).unwrap();
        fs::write(manga_root.join("m6/001.jpg"), b"1").unwrap();
        // m7：空漫画（目录在、没有图片），不该被误报
        fs::create_dir_all(manga_root.join("m7")).unwrap();

        let mut idx = LibraryIndex::default();
        idx.items.push(manga_titled("m1", "甲", &["001.jpg", "002.jpg"]));
        idx.items.push(manga_titled("m2", "乙", &["001.jpg"]));
        idx.items.push(manga_titled("m4", "丙", &["001.jpg", "002.jpg"]));
        idx.items.push(manga_titled("m5", "丙", &["001.jpg", "002.jpg"]));
        idx.items.push(manga_titled("m6", "丁", &["001.jpg", "002.jpg"]));
        idx.items.push(manga_titled("m7", "空", &[]));

        let report = build_health_report(&root, &idx);
        assert_eq!(report.indexed, 6);
        assert_eq!(report.on_disk, 6); // m1 m3 m4 m5 m6 m7

        // 索引有条目、目录丢了
        let missing: Vec<&str> = report.missing_dirs.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(missing, vec!["m2"]);
        assert_eq!(report.missing_dirs[0].pages, 1);

        // 目录在、索引没有；并报出磁盘实况（3 个文件里 2 张图）
        let orphans: Vec<&str> = report.orphan_dirs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(orphans, vec!["m3"]);
        assert_eq!(report.orphan_dirs[0].files, 3);
        assert_eq!(report.orphan_dirs[0].images, 2);
        assert_eq!(report.orphan_dirs[0].bytes, 1 + 2 + 3);
        assert!(report.orphan_dirs[0].title.is_none());

        // 疑似重复：页面集合完全一致
        assert_eq!(report.duplicates.len(), 1);
        let group = &report.duplicates[0];
        assert_eq!(group.title, "丙");
        assert_eq!(group.reason, "pages_identical");
        let members: Vec<&str> = group.members.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(members, vec!["m4", "m5"]);
        assert!(group.members.iter().all(|m| m.images == 2 && !m.dir_missing));

        // 页数漂移：m6 记 2 页、磁盘 1 张
        assert_eq!(report.page_drift.len(), 1);
        assert_eq!(report.page_drift[0].id, "m6");
        assert_eq!(report.page_drift[0].indexed, 2);
        assert_eq!(report.page_drift[0].on_disk, 1);

        // 磁盘实况：m1 两个文件、文件名样本、标题来自索引
        let m1 = report
            .disk_stats
            .iter()
            .find(|s| s.id == "m1")
            .expect("m1 必须出现在磁盘实况里");
        assert_eq!((m1.files, m1.images, m1.bytes), (2, 2, 5 + 3));
        assert_eq!(m1.sample, vec!["001.jpg", "002.jpg"]);
        assert_eq!(m1.title.as_deref(), Some("甲"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resync_manga_from_disk_repairs_pages_cover_and_thumb() {
        let root = temp_dir("resync");
        let dir = root.join(MANGA_DIR_NAME).join("m1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("001.jpg"), b"1").unwrap();
        fs::write(dir.join("002.jpg"), b"2").unwrap();
        fs::write(dir.join("003.jpg"), b"3").unwrap();

        let mut item = manga_titled("m1", "甲", &["001.jpg", "002.jpg"]);
        assert!(resync_manga_from_disk(&root, &mut item));
        let meta = item.manga.as_ref().unwrap();
        // 磁盘上多出来的第三页补进索引
        assert_eq!(meta.pages, vec!["001.jpg", "002.jpg", "003.jpg"]);
        assert_eq!(meta.cover.as_deref(), Some("001.jpg")); // 封面页还在，保留

        // 封面页被删掉 → 顺延到第一页
        fs::remove_file(dir.join("001.jpg")).unwrap();
        assert!(resync_manga_from_disk(&root, &mut item));
        let meta = item.manga.as_ref().unwrap();
        assert_eq!(meta.pages, vec!["002.jpg", "003.jpg"]);
        assert_eq!(meta.cover.as_deref(), Some("002.jpg"));

        // 图片清空 → 页数与封面一起置空
        fs::remove_file(dir.join("002.jpg")).unwrap();
        fs::remove_file(dir.join("003.jpg")).unwrap();
        assert!(resync_manga_from_disk(&root, &mut item));
        let meta = item.manga.as_ref().unwrap();
        assert!(meta.pages.is_empty());
        assert!(meta.cover.is_none());
        assert!(meta.thumb.is_none());

        // 已经和磁盘一致时返回 false（调用方不必落盘）
        assert!(!resync_manga_from_disk(&root, &mut item));

        // 文件夹没有 manga 元数据，直接返回 false
        let mut folder_item = folder("f1", None);
        assert!(!resync_manga_from_disk(&root, &mut folder_item));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn adopt_manga_dir_registers_orphan_directory() {
        let root = temp_dir("adopt");
        // 磁盘上有一份没被索引记下的目录（导入死在写索引前）
        let dir = root.join(MANGA_DIR_NAME).join("m3");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("002.jpg"), b"2").unwrap();
        fs::write(dir.join("001.jpg"), b"1").unwrap();
        fs::write(dir.join("readme.txt"), b"x").unwrap();

        let mut idx = LibraryIndex::default();
        assert_eq!(idx.next_seq, 1);
        let item = idx
            .adopt_manga_dir(&root, "m3", "失而复得", Some("f1"))
            .expect("应能收进书架");

        // id 沿用目录名，页数 = 磁盘上的图片（txt 不算），封面取第一页
        assert_eq!(item.id, "m3");
        assert_eq!(item.parent.as_deref(), Some("f1"));
        let meta = item.manga.as_ref().unwrap();
        assert_eq!(meta.dir, "manga/m3");
        assert_eq!(meta.pages, vec!["001.jpg", "002.jpg"]);
        assert_eq!(meta.cover.as_deref(), Some("001.jpg"));
        assert_eq!(meta.progress, 0.0);
        // 假图片解不出缩略图，回退原图封面（与导入路径一致）
        assert!(meta.thumb.is_none());
        assert_eq!(idx.item("m3").unwrap().title, "失而复得");
        assert_eq!(idx.children(Some("f1")).len(), 1);

        // 计数器抬到 m3 之上：之后的导入不会再来抢 m3 这个目录
        assert_eq!(idx.next_seq, 4);
        assert_eq!(idx.next_id(ItemKind::Manga), "m4");
    }

    #[test]
    fn adopt_manga_dir_rejects_conflicts_and_empty_dirs() {
        let root = temp_dir("adopt-bad");
        let manga_root = root.join(MANGA_DIR_NAME);

        // 空目录：没复制完的残骸，不许收
        fs::create_dir_all(manga_root.join("m1")).unwrap();
        let mut idx = LibraryIndex::default();
        let err = idx
            .adopt_manga_dir(&root, "m1", "空", None)
            .expect_err("空目录不该被收进书架");
        assert!(err.contains("没有图片"), "实际: {err}");

        // id 已被占用
        let mut with_item = LibraryIndex::default();
        with_item.items.push(manga("m2", None));
        let err = with_item
            .adopt_manga_dir(&root, "m2", "重名", None)
            .expect_err("id 已存在");
        assert!(err.contains("已被占用"), "实际: {err}");

        // 目录名撞车：索引里 m9 指着 manga/m3，不能把 m3 再收一遍
        let mut odd = LibraryIndex::default();
        let mut m9 = manga("m9", None);
        m9.manga.as_mut().unwrap().dir = "manga/m3".to_string();
        odd.items.push(m9);
        fs::create_dir_all(manga_root.join("m3")).unwrap();
        fs::write(manga_root.join("m3/001.jpg"), b"1").unwrap();
        let err = odd
            .adopt_manga_dir(&root, "m3", "撞车", None)
            .expect_err("目录已被别的条目占用");
        assert!(err.contains("已被索引里的条目占用"), "实际: {err}");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn health_report_ignores_same_title_with_different_pages() {
        let root = temp_dir("health-same-title");
        let manga_root = root.join(MANGA_DIR_NAME);
        for id in ["m1", "m2"] {
            fs::create_dir_all(manga_root.join(id)).unwrap();
            fs::write(manga_root.join(format!("{id}/001.jpg")), b"1").unwrap();
        }
        // 同名但页数不同：更可能是用户故意建的两部，不报重复
        let mut idx = LibraryIndex::default();
        idx.items.push(manga_titled("m1", "同名", &["001.jpg"]));
        idx.items.push(manga_titled("m2", "同名", &["001.jpg", "002.jpg"]));

        let report = build_health_report(&root, &idx);
        assert!(report.duplicates.is_empty());
        assert!(report.missing_dirs.is_empty());
        assert!(report.orphan_dirs.is_empty());

        fs::remove_dir_all(&root).ok();
    }
}
