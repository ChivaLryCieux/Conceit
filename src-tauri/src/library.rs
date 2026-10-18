//! Conceit 书库模型：索引结构、持久化与树操作。
//!
//! 书库目录位于 `<app_data_dir>/library`：
//! - `index.json`      书架索引（条目 + 每个容器内的排序）
//! - `manga/<id>/`     每部漫画导入时复制进来的图片

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const INDEX_VERSION: u32 = 1;
pub const INDEX_FILE_NAME: &str = "index.json";
pub const MANGA_DIR_NAME: &str = "manga";

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

    pub fn contains(&self, id: &str) -> bool {
        self.items.iter().any(|i| i.id == id)
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

/// 判断扩展名是否为图片
pub fn is_image_file(path: &Path) -> bool {
    path.extension()
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

/// 从 zip 压缩包中解压出图片内容
pub fn extract_zip(src: &Path, dst: &Path) -> Result<(), String> {
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
