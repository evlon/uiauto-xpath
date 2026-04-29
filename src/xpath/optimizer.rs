use crate::error::{Result, XPathError};
/// XPath 优化器
///
/// 核心思路：
/// 1. 解析完整路径为节点列表
/// 2. 对每个节点评分，找到最优"锚点"（唯一性最强的节点）
/// 3. 用 // 跳过锚点之前的所有节点
/// 4. 从锚点到目标之间：若中间节点少则保留，多则再用 // 跳过
/// 5. 对目标节点的属性进行精简和模糊化处理


// ──────────────────────────────────────────────
// 公开配置
// ──────────────────────────────────────────────

/// 优化选项
#[derive(Debug, Clone)]
pub struct OptimizeOptions {
    /// 动态 ClassName（含随机后缀）改用 starts-with()
    pub dynamic_class_to_starts_with: bool,
    /// 去除 FrameworkId（对定位通常无帮助）
    pub remove_framework_id: bool,
    /// 去除冗余的 ControlType（当 tag 已经表达类型时）
    pub remove_redundant_control_type: bool,
    /// 锚点到目标之间，中间节点数量超过此阈值则用 // 跳过
    pub max_intermediate_steps: usize,
    /// 目标节点保留 Name 属性的最大长度（超过则认为是动态标题，丢弃）
    pub max_name_length_in_target: usize,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self {
            dynamic_class_to_starts_with: true,
            remove_framework_id: true,
            remove_redundant_control_type: true,
            max_intermediate_steps: 2,
            max_name_length_in_target: 30,
        }
    }
}

/// 优化结果
#[derive(Debug, Clone)]
pub struct OptimizeResult {
    /// 主推荐：锚点 + 相对路径
    pub anchor_relative: String,
    /// 备选：最短绝对路径（仅保留分值最高的两个节点）
    pub minimal: String,
    /// 被选为锚点的节点描述（调试用）
    pub anchor_desc: String,
    /// 压缩率 0.0~1.0
    pub compression_ratio: f64,
}

// ──────────────────────────────────────────────
// 内部数据结构
// ──────────────────────────────────────────────

/// 解析后的单个节点
#[derive(Debug, Clone)]
struct ParsedNode {
    tag: String,
    /// 属性列表，保持原始顺序
    attrs: Vec<(String, String)>,
}

impl ParsedNode {
    fn get_attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

// ──────────────────────────────────────────────
// 主入口
// ──────────────────────────────────────────────

/// 对完整 XPath 进行优化，返回简洁高效的等价表达式。
///
/// # 示例
/// ```
/// use crate::{optimize, OptimizeOptions};
///
/// let full = r#"//Pane[@ControlType='Pane' and @ClassName='ChromeWidgetWin1']/...很长.../Group[@ClassName='temp-dialogue-btn...']"#;
/// let result = optimize(full, &OptimizeOptions::default()).unwrap();
/// println!("{}", result.anchor_relative);
/// // => //Document[@AutomationId='RootWebArea']//Group[starts-with(@ClassName,'temp-dialogue-btn')]
/// ```
pub fn optimize(xpath: &str, opts: &OptimizeOptions) -> Result<OptimizeResult> {
    let nodes = parse_xpath(xpath)?;
    if nodes.is_empty() {
        return Err(XPathError::ParseError("empty xpath".into()));
    }

    let scores: Vec<u32> = nodes.iter().map(|n| anchor_score(n)).collect();
    let target_idx = nodes.len() - 1;

    // 找最高分锚点（不能是目标节点本身）
    let anchor_idx = if nodes.len() > 1 {
        scores[..target_idx]
            .iter()
            .enumerate()
            .max_by_key(|(_, &s)| s)
            .map(|(i, _)| i)
    } else {
        None
    };

    let anchor_relative = build_anchor_relative(&nodes, anchor_idx, target_idx, opts);
    let minimal = build_minimal(&nodes, &scores, target_idx, opts);

    let anchor_desc = anchor_idx
        .map(|i| format!("{}[{}] (score={})", nodes[i].tag, i + 1, scores[i]))
        .unwrap_or_else(|| "none".into());

    let compression_ratio = 1.0 - (anchor_relative.len() as f64 / xpath.len() as f64);

    Ok(OptimizeResult {
        anchor_relative,
        minimal,
        anchor_desc,
        compression_ratio,
    })
}

// ──────────────────────────────────────────────
// 构建输出
// ──────────────────────────────────────────────

/// 策略一：// 跳过锚点前所有节点，从锚点相对定位到目标
fn build_anchor_relative(
    nodes: &[ParsedNode],
    anchor_idx: Option<usize>,
    target_idx: usize,
    opts: &OptimizeOptions,
) -> String {
    match anchor_idx {
        None => {
            // 没有好锚点，直接 // target
            format!("//{}", render_node(&nodes[target_idx], true, opts))
        }
        Some(ai) => {
            let anchor_str = format!("//{}", render_node(&nodes[ai], false, opts));
            let mid_count = target_idx - ai - 1;
            let target_str = render_node(&nodes[target_idx], true, opts);

            if mid_count == 0 {
                // 锚点就是目标的直接父节点
                format!("{}/{}", anchor_str, target_str)
            } else if mid_count <= opts.max_intermediate_steps {
                // 中间节点较少，逐个列出（仅保留 tag，去掉属性以保持简洁）
                let mid: String = nodes[ai + 1..target_idx]
                    .iter()
                    .map(|n| n.tag.clone())
                    .collect::<Vec<_>>()
                    .join("/");
                format!("{}/{}/{}", anchor_str, mid, target_str)
            } else {
                // 中间节点较多，用 // 再跳一次
                format!("//{}", render_node(&nodes[ai], false, opts))
                    + "//"
                    + &target_str
            }
        }
    }
}

/// 策略二：只保留分值最高的前驱 + 目标，用 // 连接
fn build_minimal(
    nodes: &[ParsedNode],
    scores: &[u32],
    target_idx: usize,
    opts: &OptimizeOptions,
) -> String {
    // 找到目标之前分值最高的节点（可能和 anchor 相同）
    let best_ancestor = if target_idx > 0 {
        scores[..target_idx]
            .iter()
            .enumerate()
            .filter(|(_, &s)| s > 0)
            .max_by_key(|(_, &s)| s)
            .map(|(i, _)| i)
    } else {
        None
    };

    let target_str = render_node(&nodes[target_idx], true, opts);
    match best_ancestor {
        Some(ai) => {
            let anc_str = render_node(&nodes[ai], false, opts);
            format!("//{}//{}", anc_str, target_str)
        }
        None => format!("//{}", target_str),
    }
}

// ──────────────────────────────────────────────
// 节点渲染
// ──────────────────────────────────────────────

/// 将节点渲染为 XPath 步骤字符串
fn render_node(node: &ParsedNode, is_target: bool, opts: &OptimizeOptions) -> String {
    let attrs = select_attrs(node, is_target, opts);
    if attrs.is_empty() {
        return node.tag.clone();
    }
    let pred = attrs.join(" and ");
    format!("{}[{}]", node.tag, pred)
}

/// 根据策略选择保留哪些属性，并对动态值做模糊化处理
fn select_attrs(node: &ParsedNode, is_target: bool, opts: &OptimizeOptions) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();

    // AutomationId —— 最高优先，直接保留
    if let Some(v) = node.get_attr("AutomationId") {
        if !v.is_empty() {
            parts.push(format!("@AutomationId='{}'", v));
            // AutomationId 已经足够唯一，对于锚点节点可以直接返回
            if !is_target {
                return parts;
            }
        }
    }

    // ControlType —— 对于非通用类型保留
    if !opts.remove_redundant_control_type {
        if let Some(ct) = node.get_attr("ControlType") {
            parts.push(format!("@ControlType='{}'", ct));
        }
    } else if let Some(ct) = node.get_attr("ControlType") {
        // 只有非通用类型才保留（Pane/Group 太泛）
        if !is_generic_control_type(ct) {
            parts.push(format!("@ControlType='{}'", ct));
        }
    }

    // ClassName —— 检测动态性
    if let Some(cn) = node.get_attr("ClassName") {
        if !cn.is_empty() {
            if opts.dynamic_class_to_starts_with && is_dynamic_class(cn) {
                let prefix = extract_stable_prefix(cn);
                if prefix.len() >= 4 {
                    parts.push(format!("starts-with(@ClassName, '{}')", prefix));
                }
                // 前缀太短则跳过（噪音）
            } else {
                parts.push(format!("@ClassName='{}'", cn));
            }
        }
    }

    // Name —— 目标节点保留（但截断过长的动态标题），锚点节点视分值决定
    if let Some(name) = node.get_attr("Name") {
        let limit = if is_target { opts.max_name_length_in_target } else { 20 };
        if !name.is_empty() && name.len() <= limit {
            parts.push(format!("@Name='{}'", name));
        }
    }

    // FrameworkId —— 通常无助定位
    if !opts.remove_framework_id {
        if let Some(fw) = node.get_attr("FrameworkId") {
            parts.push(format!("@FrameworkId='{}'", fw));
        }
    }

    parts
}

// ──────────────────────────────────────────────
// 锚点评分
// ──────────────────────────────────────────────

/// 节点唯一性评分。分值越高越适合作锚点。
fn anchor_score(node: &ParsedNode) -> u32 {
    let mut score = 0u32;

    // AutomationId：开发者明确设置，全局唯一性最强
    if let Some(v) = node.get_attr("AutomationId") {
        if !v.is_empty() { score += 10; }
    }

    // Name：有语义且通常稳定，但页面标题类会动态变化
    if let Some(v) = node.get_attr("Name") {
        if !v.is_empty() && v.len() <= 40 { score += 4; }
    }

    // ClassName：稳定的类名加分，动态的不加
    if let Some(cn) = node.get_attr("ClassName") {
        if !cn.is_empty() && !is_dynamic_class(cn) { score += 4; }
    }

    // ControlType：非通用类型加分
    if let Some(ct) = node.get_attr("ControlType") {
        if !is_generic_control_type(ct) { score += 3; }
    }

    // Tag 本身的语义：Document/Edit/Button 等比 Pane/Group 更具唯一性
    score += tag_uniqueness_bonus(&node.tag);

    score
}

/// 根据 tag 名额外加分
fn tag_uniqueness_bonus(tag: &str) -> u32 {
    match tag {
        "Document" => 5,
        "Edit" | "Button" | "CheckBox" | "RadioButton" => 4,
        "List" | "ListItem" | "TreeItem" | "ComboBox" => 3,
        "Window" | "Dialog" => 3,
        "Text" | "Image" => 1,
        "Pane" | "Group" => 0,
        _ => 1,
    }
}

// ──────────────────────────────────────────────
// 辅助判断
// ──────────────────────────────────────────────

/// 判断 ControlType 是否属于泛型（不具备良好区分度）
fn is_generic_control_type(ct: &str) -> bool {
    matches!(ct, "Pane" | "Group" | "Custom")
}

/// 判断 ClassName 是否包含动态后缀（随机字母数字组合）
///
/// 启发式规则（任一满足即视为动态）：
/// 1. 包含超过 2 个空格分隔的 token（多段动态 class，如 React CSS Modules）
/// 2. 任意 token 末尾有 4+ 位纯数字（如 `btn123456`）
/// 3. 任意 token 含有 camelCase 突然夹杂大写+数字混合段（如 `BOp4`, `ZJ07f`, `wilLn`）
/// 4. 任意 token 长度超过 25（通常是编译后的混淆名）
fn is_dynamic_class(cn: &str) -> bool {
    let tokens: Vec<&str> = cn.split_whitespace().collect();
    // 规则 1：超过 2 个 token（三段以上组合类名几乎都是动态的）
    if tokens.len() > 2 {
        return true;
    }
    for token in &tokens {
        // 规则 4：超长 token
        if token.len() > 25 {
            return true;
        }
        // 规则 2 & 3：token 内部有随机段
        if token_has_random_segment(token) {
            return true;
        }
    }
    false
}

/// 检测单个 token 是否含有随机段。
///
/// 随机段特征：在 camelCase 或连字符分隔词之后出现
/// 大小写字母与数字的混合（camelCase hash），例如：
/// - `btnBOp4`  → `BOp4` 是随机段（全大写起头 + 小写 + 数字）
/// - `PagewilLn` → `wilLn` 是随机段（小写+大写+小写 交替）
/// - `optionsZJ07f` → `ZJ07f` 是随机段
///
/// 算法：找到 token 中最后一段"驼峰词"，判断它是否呈现随机特征
fn token_has_random_segment(s: &str) -> bool {
    // 按 `-` 分割，检查每个 camelCase 段
    for part in s.split('-') {
        if camel_segment_is_random(part) {
            return true;
        }
    }
    false
}

/// 对 camelCase 字符串的最后一个"词"（大写字母起始的连续段）判断是否随机
fn camel_segment_is_random(s: &str) -> bool {
    let words = split_camel(s);
    if words.len() < 2 {
        return false;
    }
    let last = words.last().unwrap();
    is_random_word(last)
}

/// 拆分 camelCase 为词列表，按大写字母边界分割
/// 例如 "btnBOp4" -> ["btn", "BOp4"]，"MultiContentsView" -> ["Multi", "Contents", "View"]
fn split_camel(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut starts: Vec<usize> = vec![0];
    for i in 1..bytes.len() {
        if bytes[i].is_ascii_uppercase() && bytes[i-1].is_ascii_lowercase() {
            starts.push(i);
        }
    }
    starts.push(s.len());
    starts.windows(2).map(|w| &s[w[0]..w[1]]).collect()
}

/// 判断一个词是否像随机生成的：
/// - 包含数字
/// - 或大小写字母无规律混合（非首字母大写模式）
/// - 或纯大写且长度 >= 2
fn is_random_word(w: &str) -> bool {
    if w.len() < 2 { return false; }
    let has_digit  = w.chars().any(|c| c.is_ascii_digit());
    let has_upper  = w.chars().any(|c| c.is_ascii_uppercase());
    let _has_lower = w.chars().any(|c| c.is_ascii_lowercase());
    let all_upper  = w.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());

    // 纯大写（如 "ZJ"）或 大写+小写+数字混合（如 "Op4", "wil", "Ln"）
    if all_upper && w.len() >= 2 { return true; }
    if has_digit && has_upper { return true; }
    // 小写中夹大写（非首字母大写，如 "wil" 后面紧跟 "Ln"）已在 split_camel 里处理
    false
}

/// 提取 ClassName 的稳定前缀
///
/// 策略：
/// 1. 取第一个空格分隔 token
/// 2. 按连字符分割，对每个 part 用 camelCase 拆分找到稳定词
/// 3. 遇到随机词立即截断，只保留之前的稳定部分
fn extract_stable_prefix(cn: &str) -> String {
    let first_token = cn.split_whitespace().next().unwrap_or(cn);

    let mut result_parts: Vec<String> = Vec::new();

    for part in first_token.split('-') {
        let words = split_camel(part);
        if words.len() <= 1 {
            // 单词 part：判断整个 part 是否随机
            if !result_parts.is_empty() && is_random_word(part) {
                break; // 整个 part 是随机的，截断
            }
            result_parts.push(part.to_string());
        } else {
            // 多词 camelCase part：找到第一个随机词，截断到那里
            let mut stable_words: Vec<String> = Vec::new();
            for word in &words {
                if is_random_word(word) {
                    break;
                }
                stable_words.push(word.to_string());
            }
            let stable_part = stable_words.join("");
            if stable_part.is_empty() {
                break;
            }
            result_parts.push(stable_part);
            // 如果 stable_words 比 words 短，说明这个 part 里有随机词，后面也不要了
            if stable_words.len() < words.len() {
                break;
            }
        }
    }

    let prefix = result_parts.join("-");
    if prefix.len() >= 4 {
        prefix
    } else {
        first_token.chars().take(20).collect()
    }
}

// ──────────────────────────────────────────────
// XPath 解析（仅解析 step/predicate 层，不依赖完整 AST）
// ──────────────────────────────────────────────

/// 将 XPath 字符串解析为节点列表。
/// 支持格式：`//Tag1[@a='v' and @b='v2']/Tag2[...]/...`
fn parse_xpath(xpath: &str) -> Result<Vec<ParsedNode>> {
    // 去掉开头的 // 或 /
    let stripped = xpath.trim_start_matches('/');

    // 按 / 分割（跳过 [ ] 内的 /）
    let raw_steps = split_steps(stripped);

    raw_steps.iter()
        .filter(|s| !s.is_empty())
        .map(|s| parse_step_str(s))
        .collect()
}

/// 按 `/` 分割路径步骤，忽略 `[...]` 内部的 `/`
fn split_steps(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;

    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' => { depth += 1; cur.push(c); }
            ']' => { depth = depth.saturating_sub(1); cur.push(c); }
            '/' if depth == 0 => {
                // 连续两个 // 表示 descendant-or-self，直接跳过（已用 // 处理）
                if chars.peek() == Some(&'/') {
                    chars.next();
                }
                if !cur.is_empty() {
                    parts.push(cur.clone());
                    cur.clear();
                }
            }
            _ => { cur.push(c); }
        }
    }
    if !cur.is_empty() { parts.push(cur); }
    parts
}

/// 解析单个步骤字符串，例如：`Pane[@ControlType='Pane' and @ClassName='Foo']`
fn parse_step_str(s: &str) -> Result<ParsedNode> {
    let s = s.trim();

    // 找 tag（第一个 `[` 前的内容）
    let (tag, rest) = if let Some(pos) = s.find('[') {
        (s[..pos].trim().to_string(), &s[pos..])
    } else {
        (s.to_string(), "")
    };

    let attrs = parse_predicates(rest)?;
    Ok(ParsedNode { tag, attrs })
}

/// 解析谓词块 `[@a='v' and @b='v2' and ...]`，提取所有 `@key='value'` 对
///
/// 只处理简单的 `@attr='value'` 形式（等号赋值），
/// 不解析 contains/starts-with 等函数（优化器本身会生成它们）。
fn parse_predicates(s: &str) -> Result<Vec<(String, String)>> {
    let mut attrs = Vec::new();

    // 用正则-style 手工扫描：找所有 @key='value' 或 @key="value"
    let mut i = 0;
    let bytes = s.as_bytes();
    let len = s.len();

    while i < len {
        // 找 @
        if bytes[i] != b'@' { i += 1; continue; }
        i += 1; // skip @

        // 读 key（字母数字下划线）
        let key_start = i;
        while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
        if i == key_start { continue; }
        let key = s[key_start..i].to_string();

        // 跳过空白
        while i < len && bytes[i] == b' ' { i += 1; }

        // 必须是 =
        if i >= len || bytes[i] != b'=' { continue; }
        i += 1;

        // 跳过空白
        while i < len && bytes[i] == b' ' { i += 1; }

        // 读引号内的值
        if i >= len || (bytes[i] != b'\'' && bytes[i] != b'"') { continue; }
        let quote = bytes[i];
        i += 1;
        let val_start = i;
        while i < len && bytes[i] != quote { i += 1; }
        let value = s[val_start..i].to_string();
        if i < len { i += 1; } // skip closing quote

        attrs.push((key, value));
    }

    Ok(attrs)
}

// ──────────────────────────────────────────────
// 测试
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "//Pane[@ControlType='Pane' and @ClassName='ChromeWidgetWin1' and @Name='元宝 - 轻松工作 多点生活 | Chat' and @FrameworkId='Win32']/Pane[@ControlType='Pane' and @ClassName='BrowserRootView' and @Name='元宝 - 轻松工作 多点生活 | Chat - Web 内容' and @FrameworkId='Chrome']/Pane[@ControlType='Pane' and @ClassName='NonClientView' and @FrameworkId='Chrome']/Pane[@ControlType='Pane' and @ClassName='EmbeddedBrowserFrameView' and @FrameworkId='Chrome']/Pane[@ControlType='Pane' and @ClassName='BrowserView' and @FrameworkId='Chrome']/Pane[@ControlType='Pane' and @ClassName='SidebarContentsSplitView' and @FrameworkId='Chrome']/Pane[@ControlType='Pane' and @ClassName='SidebarContentsSplitView' and @FrameworkId='Chrome']/Pane[@ControlType='Pane' and @ClassName='View' and @FrameworkId='Chrome']/Pane[@ControlType='Pane' and @ClassName='MultiContentsView' and @FrameworkId='Chrome']/Pane[@ControlType='Pane' and @ClassName='View' and @FrameworkId='Chrome']/Document[@ControlType='Document' and @AutomationId='RootWebArea' and @Name='元宝 - 轻松工作 多点生活 | Chat' and @FrameworkId='Chrome']/Group[@ControlType='Group' and @FrameworkId='Chrome']/Group[@ControlType='Group' and @ClassName='chatmainPagewilLn mainPageCtrl chatmainPageWinyRJfh' and @FrameworkId='Chrome']/Group[@ControlType='Group' and @ClassName='temp-dialogue-btntemp-dialogue-btnBOp4 winFolder_optionsZJ07f t-popup-open' and @FrameworkId='Chrome']";

    #[test]
    fn test_parse_steps() {
        let nodes = parse_xpath(EXAMPLE).unwrap();
        assert_eq!(nodes.len(), 14);
        assert_eq!(nodes[0].tag, "Pane");
        assert_eq!(nodes[10].tag, "Document");
        assert_eq!(
            nodes[10].get_attr("AutomationId"),
            Some("RootWebArea")
        );
    }

    #[test]
    fn test_anchor_score() {
        let nodes = parse_xpath(EXAMPLE).unwrap();
        // Document 节点有 AutomationId，应得分最高
        let scores: Vec<u32> = nodes.iter().map(|n| anchor_score(n)).collect();
        let doc_idx = 10;
        let max_score = *scores.iter().max().unwrap();
        assert_eq!(scores[doc_idx], max_score,
            "Document node should have highest score, scores={:?}", scores);
    }

    #[test]
    fn test_is_dynamic_class() {
        assert!(is_dynamic_class("temp-dialogue-btnBOp4 winFolder_optionsZJ07f t-popup-open"));
        assert!(is_dynamic_class("chatmainPagewilLn mainPageCtrl chatmainPageWinyRJfh"));
        assert!(!is_dynamic_class("BrowserRootView"));
        assert!(!is_dynamic_class("NonClientView"));
    }

    #[test]
    fn test_extract_stable_prefix() {
        // "temp-dialogue-btnBOp4" -> "temp-dialogue-btn"
        let p = extract_stable_prefix("temp-dialogue-btnBOp4 winFolder_optionsZJ07f t-popup-open");
        assert_eq!(p, "temp-dialogue-btn");

        // "BrowserRootView" 不含随机段，返回原值
        let p2 = extract_stable_prefix("BrowserRootView");
        assert_eq!(p2, "BrowserRootView");
    }

    #[test]
    fn test_optimize_example() {
        let opts = OptimizeOptions::default();
        let result = optimize(EXAMPLE, &opts).unwrap();

        println!("anchor_relative : {}", result.anchor_relative);
        println!("minimal         : {}", result.minimal);
        println!("anchor_desc     : {}", result.anchor_desc);
        println!("compression     : {:.1}%", result.compression_ratio * 100.0);

        // 锚点应包含 AutomationId='RootWebArea'
        assert!(result.anchor_relative.contains("AutomationId='RootWebArea'"),
            "anchor_relative={}", result.anchor_relative);

        // 目标应使用 starts-with(@ClassName, ...)，前缀是去掉随机后缀后的稳定部分
        // 实际 ClassName 为 "temp-dialogue-btntemp-dialogue-btnBOp4 ..."
        // 稳定前缀 = "temp-dialogue-btntemp-dialogue-btn"
        assert!(result.anchor_relative.contains("starts-with(@ClassName,"),
            "anchor_relative={}", result.anchor_relative);

        // 压缩率应大于 70%
        assert!(result.compression_ratio > 0.7,
            "compression_ratio={}", result.compression_ratio);
    }

    #[test]
    fn test_no_anchor() {
        // 只有两个节点，都是通用 Pane
        let simple = "//Pane[@ControlType='Pane']/Pane[@ControlType='Pane' and @ClassName='Target']";
        let result = optimize(simple, &OptimizeOptions::default()).unwrap();
        println!("simple result: {}", result.anchor_relative);
        // 应该能正常输出，不 panic
    }
}
