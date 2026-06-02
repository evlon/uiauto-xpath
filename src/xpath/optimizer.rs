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
    /// 锚点节点索引（在原始节点列表中的位置）
    pub anchor_index: Option<usize>,
    /// 目标节点索引（在原始节点列表中的位置）
    pub target_index: usize,
    /// 压缩率 0.0~1.0
    pub compression_ratio: f64,
    /// 简化属性数量（使用 starts-with/ends-with/contains 的数量）
    pub simplified_attrs_count: usize,
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
/// ```ignore
/// use uiauto_xpath::{optimize, OptimizeOptions};
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
    
    // 统计简化属性数量（starts-with 的使用次数）
    let simplified_attrs_count = anchor_relative.matches("starts-with").count()
        + anchor_relative.matches("ends-with").count()
        + anchor_relative.matches("contains").count();

    Ok(OptimizeResult {
        anchor_relative,
        minimal,
        anchor_desc,
        anchor_index: anchor_idx,
        target_index: target_idx,
        compression_ratio,
        simplified_attrs_count,
    })
}

// ──────────────────────────────────────────────
// 构建输出
// ──────────────────────────────────────────────

/// 策略一：// 跳过锚点前所有节点，从锚点相对定位到目标
/// 特殊情况：锚点是第一个节点时，用 / 开头（绝对路径）
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
        Some(0) => {
            // 锚点是第一个节点（根节点），用绝对路径 / 开头
            let anchor_str = format!("/{}", render_node(&nodes[0], false, opts));
            let target_str = render_node(&nodes[target_idx], true, opts);
            
            if target_idx == 0 {
                // 目标就是锚点（根节点本身）
                anchor_str
            } else if target_idx == 1 {
                // 目标是锚点的直接子节点
                format!("{}/{}", anchor_str, target_str)
            } else {
                // 有中间节点
                let mid_count = target_idx - 1;
                if mid_count <= opts.max_intermediate_steps {
                    let mid: String = nodes[1..target_idx]
                        .iter()
                        .map(|n| n.tag.clone())
                        .collect::<Vec<_>>()
                        .join("/");
                    format!("{}/{}/{}", anchor_str, mid, target_str)
                } else {
                    // 中间节点多，用 // 跳过
                    format!("{}//{}", anchor_str, target_str)
                }
            }
        }
        Some(ai) => {
            // 锚点不是第一个节点，用相对路径 // 开头（跳过锚点前的节点）
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

    // AutomationId —— 最高优先，直接保留（但跳过纯数字 ID，大概率是随机生成的）
    if let Some(v) = node.get_attr("AutomationId") {
        if !v.is_empty() && !is_numeric_aid(v) {
            parts.push(format!("@AutomationId='{}'", v));
            // AutomationId 已经足够唯一，对于锚点节点可以直接返回
            if !is_target {
                return parts;
            }
        }
    }

    // ControlType 已通过 XPath 标签名表达，不再添加谓词
    // 完全移除 ControlType 谓词以避免冗余

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
        // 使用字符数而非字节数，避免中文/emoji 等 UTF-8 多字节字符被误判过长
        if !name.is_empty() && name.chars().count() <= limit {
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

    // AutomationId：开发者明确设置，全局唯一性最强（纯数字 ID 视为随机，不加分）
    if let Some(v) = node.get_attr("AutomationId") {
        if !v.is_empty() && !is_numeric_aid(v) { score += 10; }
    }

    // Name：有语义且通常稳定，但页面标题类会动态变化
    if let Some(v) = node.get_attr("Name") {
        if !v.is_empty() && v.len() <= 40 { score += 4; }
    }

    // ClassName：稳定的类名加分，动态的不加
    if let Some(cn) = node.get_attr("ClassName") {
        if !cn.is_empty() && !is_dynamic_class(cn) { score += 4; }
    }

    // ControlType 评分转移到 tag_uniqueness_bonus，此处不再单独评分
    // 因为 ControlType 已通过标签名体现

    // Tag 本身的语义：Document/Edit/Button 等比 Pane/Group 更具唯一性
    score += tag_uniqueness_bonus(&node.tag);

    score
}

/// 根据 tag 名额外加分
pub fn tag_uniqueness_bonus(tag: &str) -> u32 {
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

/// 判断是否为纯数字 automation_id（大概率是随机生成的）
fn is_numeric_aid(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// 判断 ControlType 是否属于泛型（不具备良好区分度）
pub fn is_generic_control_type(ct: &str) -> bool {
    matches!(ct, "Pane" | "Group" | "Custom")
}

/// 判断 ClassName 是否包含动态后缀（随机字母数字组合）
///
/// 启发式规则（任一满足即视为动态）：
/// 1. 包含超过 2 个空格分隔的 token（多段动态 class，如 React CSS Modules）
/// 2. 任意 token 末尾有 4+ 位纯数字（如 `btn123456`）
/// 3. 任意 token 含有 camelCase 突然夹杂大写+数字混合段（如 `BOp4`, `ZJ07f`, `wilLn`）
/// 4. 任意 token 长度超过 25（通常是编译后的混淆名）
pub fn is_dynamic_class(cn: &str) -> bool {
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
pub fn split_camel(s: &str) -> Vec<&str> {
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
pub fn extract_stable_prefix(cn: &str) -> String {
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
/// 【关键修复】同时处理 starts-with(@ClassName, 'value') 函数调用
fn parse_predicates(s: &str) -> Result<Vec<(String, String)>> {
    let mut attrs = Vec::new();

    // 【关键修复】先查找 starts-with 函数调用
    let mut search_start = 0;
    while let Some(pos) = s[search_start..].find("starts-with(") {
        let func_pos = search_start + pos;
        
        // 找到对应的右括号
        let paren_start = func_pos + "starts-with(".len();
        let mut depth = 1;
        let mut i = paren_start;
        let bytes = s.as_bytes();
        
        while i < s.len() && depth > 0 {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        
        if depth == 0 {
            let func_content = &s[paren_start..i-1];
            
            // 解析 starts-with(@ClassName, 'value')
            // 格式：@ClassName, 'value'
            let parts: Vec<&str> = func_content.split(',').collect();
            if parts.len() >= 2 {
                let attr_part = parts[0].trim();
                let value_part = parts[1].trim();
                
                // 提取属性名（去掉 @）
                if attr_part.starts_with('@') {
                    let attr_name = &attr_part[1..];
                    
                    // 提取值（去掉引号）
                    let value = value_part.trim_matches(|c| c == '\'' || c == '"');
                    
                    attrs.push((format!("__starts_with__{}", attr_name), value.to_string()));
                }
            }
            
            search_start = i;
        } else {
            break;
        }
    }

    // 然后查找简单的 @key='value' 模式
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
// 极简优化（带取消支持）
// ──────────────────────────────────────────────

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

/// 极简优化：通过尝试验证移除所有非必要属性
/// 
/// # 参数
/// - `xpath`: 原始 XPath 字符串
/// - `verify_callback`: 验证回调函数，接收简化后的 XPath，返回是否找到元素
///   - 签名：`Fn(&str) -> Result<bool>`
///   - 返回 Ok(true) 表示该 XPath 能定位到元素
///   - 返回 Err 表示验证失败或用户取消
/// - `progress_callback`: 进度回调函数，用于输出日志
///   - 签名：`Fn(&str)`
///   - 每次尝试验证时调用，输出当前状态
/// - `cancel_flag`: 取消标志，设置为 true 时中断优化
pub fn optimize_minimal_with_cancel<F, P>(
    xpath: &str, 
    verify_callback: F,
    progress_callback: P,
    cancel_flag: Arc<AtomicBool>,
) -> Result<Option<String>>
where
    F: Fn(&str) -> Result<bool>,
    P: Fn(&str),
{
    use std::time::Instant;
    let total_start = Instant::now();
    
    progress_callback("[极简优化] 开始智能简化 XPath...");
    
    // 1. 解析 XPath 为节点列表
    let nodes = parse_xpath(xpath)?;
    if nodes.is_empty() {
        return Err(XPathError::ParseError("empty xpath".into()));
    }
    
    let target_idx = nodes.len() - 1;
    progress_callback(&format!("[极简优化] 分析 {} 个节点...", nodes.len()));
    
    // 【调试】打印解析后的节点信息
    for (i, node) in nodes.iter().enumerate() {
        progress_callback(&format!(
            "  [{}] {} - {} 个属性: {:?}",
            i,
            node.tag,
            node.attrs.len(),
            node.attrs.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>()
        ));
    }
    
    // 【关键修复】直接从原始 XPath 的节点开始，而不是从标准优化结果开始
    // 因为标准优化可能已经移除了必要的属性
    let mut optimized_nodes = nodes.clone();
    progress_callback(&format!("[极简优化] 原始 XPath 长度: {} 字符", xpath.len()));
    
    // 3. 对每个节点的每个属性进行尝试移除
    let mut attempts = 0;
    let max_attempts = 50;
    let mut removed_count = 0;
    let mut kept_count = 0;
    
    for node_idx in 0..optimized_nodes.len() {
        // 检查取消标志
        if cancel_flag.load(Ordering::SeqCst) {
            progress_callback("[极简优化] 检测到取消信号，停止优化");
            return Ok(None);
        }
        
        let node_tag = &optimized_nodes[node_idx].tag;
        let original_attrs = optimized_nodes[node_idx].attrs.clone();
        let attr_count = original_attrs.len();
        
        progress_callback(&format!(
            "\n[极简优化] 处理第 {}/{} 个节点: {}",
            node_idx + 1,
            optimized_nodes.len(),
            node_tag
        ));
        
        // 【关键修复】从完整属性开始，按优先级从低到高逐个尝试移除
        // 优先级（从低到高）：LocalizedControlType < FrameworkId < Name < ClassName < AutomationId(非数字)
        // 【重要】非纯数字 AutomationId 仍尽量保留，但会尝试移除看是否仍能匹配
        let priority_order = ["LocalizedControlType", "FrameworkId", "Name", "ClassName", "AutomationId"];
        
        // 初始状态：保留所有属性
        let mut attrs_to_keep: Vec<(String, String)> = original_attrs.clone();
        
        for attr_name in &priority_order {
            // 检查取消标志
            if cancel_flag.load(Ordering::SeqCst) {
                progress_callback("[极简优化] 检测到取消信号，停止优化");
                return Ok(None);
            }
            
            // 【关键修复】同时检查普通属性和 __starts_with__ 前缀的属性
            let attr_pos = attrs_to_keep.iter()
                .position(|(k, _)| {
                    k.eq_ignore_ascii_case(attr_name) || 
                    k.eq_ignore_ascii_case(&format!("__starts_with__{}", attr_name))
                });
            
            if attr_pos.is_none() {
                // 已经没有这个属性了，跳过
                continue;
            }
            
            let attr_pos = attr_pos.unwrap();
            let (attr_name_actual, attr_value) = attrs_to_keep[attr_pos].clone();
            
            // 【关键修复】尝试移除这个属性
            let mut test_attrs = attrs_to_keep.clone();
            test_attrs.remove(attr_pos);  // 移除该属性
            
            // 构建测试 XPath
            let test_node = ParsedNode {
                tag: optimized_nodes[node_idx].tag.clone(),
                attrs: test_attrs.clone(),
            };
            
            let test_xpath = build_test_xpath(&optimized_nodes, node_idx, &test_node);
            
            // 尝试验证
            attempts += 1;
            if attempts > max_attempts {
                progress_callback(&format!(
                    "  ⚠ 达到最大尝试次数 ({})，停止优化",
                    max_attempts
                ));
                break;
            }
            
            // let attempt_start = Instant::now();  // 不再显示耗时
            let attr_display = if attr_value.chars().count() > 30 {
                format!("{}...", attr_value.chars().take(30).collect::<String>())
            } else {
                attr_value.clone()
            };
            
            progress_callback(&format!(
                "  [尝试 {}/{}] 测试移除 @{}='{}'...",
                attempts,
                max_attempts,
                attr_name_actual,
                attr_display
            ));
            
            let verified = verify_callback(&test_xpath)?;
            // let elapsed = attempt_start.elapsed();  // 不再显示耗时
            
            if verified {
                // 验证成功（移除后仍能唯一定位），永久移除该属性
                attrs_to_keep = test_attrs;
                removed_count += 1;
                progress_callback(&format!(
                    "  ✓ 移除 {} (简化成功)",
                    attr_name_actual
                ));
            } else {
                // 验证失败（移除后不唯一或找不到），保留该属性
                kept_count += 1;
                progress_callback(&format!(
                    "  ✗ 保留 {} (必需属性)",
                    attr_name_actual
                ));
            }
        }  // for attr_name loop
        
        // 更新节点属性
        let final_attr_count = attrs_to_keep.len();
        
        if attr_count > final_attr_count {
            progress_callback(&format!(
                "  → {} 简化完成：移除 {} 个冗余属性",
                node_tag,
                attr_count - final_attr_count
            ));
        } else {
            progress_callback(&format!(
                "  → {} 所有属性均为必需，无法进一步简化",
                node_tag
            ));
        }
        
        optimized_nodes[node_idx].attrs = attrs_to_keep;
    }
    
    // 4. 重新构建最终 XPath
    let final_xpath = rebuild_xpath_from_nodes(&optimized_nodes, target_idx);
    let total_elapsed = total_start.elapsed();
    
    progress_callback(&format!(
        "\n[极简优化] 优化完成！总耗时: {:.1}秒",
        total_elapsed.as_secs_f64()
    ));
    progress_callback(&format!(
        "  - 共尝试 {} 次属性移除",
        attempts
    ));
    progress_callback(&format!(
        "  - 成功简化 {} 个属性",
        removed_count
    ));
    progress_callback(&format!(
        "  - 保留 {} 个必需属性",
        kept_count
    ));
    progress_callback(&format!(
        "  - XPath 长度: {} → {} 字符 (压缩 {:.0}%)",
        xpath.len(),
        final_xpath.len(),
        (1.0 - final_xpath.len() as f64 / xpath.len() as f64) * 100.0
    ));
    progress_callback(&format!(
        "  - 最终 XPath: {}",
        if final_xpath.len() > 100 {
            format!("{}...", &final_xpath[..100])
        } else {
            final_xpath.clone()
        }
    ));
    
    Ok(Some(final_xpath))
}

/// 构建测试用 XPath（只修改指定节点的属性）
fn build_test_xpath(
    nodes: &[ParsedNode],
    modified_idx: usize,
    modified_node: &ParsedNode,
) -> String {
    // 【关键修复】直接根据节点的实际属性构建 XPath，不使用 render_node/select_attrs
    // 因为 select_attrs 会根据优化选项过滤掉某些属性
    
    let prefix = if modified_idx == 0 { "/" } else { "//" };
    
    let parts: Vec<String> = nodes.iter().enumerate().map(|(i, node)| {
        let current_node = if i == modified_idx { modified_node } else { node };
        
        // 直接构建谓词，使用节点中的所有属性
        let mut predicates: Vec<String> = Vec::new();
        for (key, value) in &current_node.attrs {
            // 【关键修复】处理 starts-with 特殊标记
            if key.starts_with("__starts_with__") {
                let attr_name = &key["__starts_with__".len()..];
                predicates.push(format!("starts-with(@{}, '{}')", attr_name, value));
            } else {
                predicates.push(format!("@{}='{}'", key, value));
            }
        }
        
        let pred_str = if predicates.is_empty() {
            String::new()
        } else {
            format!("[{}]", predicates.join(" and "))
        };
        
        format!("{}{}", current_node.tag, pred_str)
    }).collect();
    
    // 使用 / 连接各个节点
    format!("{}{}", prefix, parts.join("/"))
}

/// 从节点列表重建 XPath
fn rebuild_xpath_from_nodes(nodes: &[ParsedNode], target_idx: usize) -> String {
    // 【关键修复】直接使用节点中的属性，不经过 select_attrs 过滤
    let prefix = if nodes.len() > 1 && target_idx > 0 { "//" } else { "/" };
    
    let parts: Vec<String> = nodes.iter().map(|node| {
        // 直接构建谓词，使用节点中的所有属性
        let mut predicates: Vec<String> = Vec::new();
        for (key, value) in &node.attrs {
            // 【关键修复】处理 starts-with 特殊标记
            if key.starts_with("__starts_with__") {
                let attr_name = &key["__starts_with__".len()..];
                predicates.push(format!("starts-with(@{}, '{}')", attr_name, value));
            } else {
                predicates.push(format!("@{}='{}'", key, value));
            }
        }
        
        let pred_str = if predicates.is_empty() {
            String::new()
        } else {
            format!("[{}]", predicates.join(" and "))
        };
        
        format!("{}{}", node.tag, pred_str)
    }).collect();
    
    // 使用 / 连接各个节点
    format!("{}{}", prefix, parts.join("/"))
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
        
        // 验证优化后的 XPath 不包含 @ControlType 谓词
        assert!(!result.anchor_relative.contains("@ControlType="),
            "Optimized XPath should not contain @ControlType predicates, got: {}", result.anchor_relative);
        assert!(!result.minimal.contains("@ControlType="),
            "Minimal XPath should not contain @ControlType predicates, got: {}", result.minimal);
    }

    #[test]
    fn test_no_anchor() {
        // 只有两个节点，都是通用 Pane
        let simple = "//Pane/Pane[@ClassName='Target']";
        let result = optimize(simple, &OptimizeOptions::default()).unwrap();
        println!("simple result: {}", result.anchor_relative);
        // 应该能正常输出，不 panic
        
        // 验证不包含 @ControlType
        assert!(!result.anchor_relative.contains("@ControlType="),
            "Should not contain @ControlType predicate");
    }
    
    // ──────────────────────────────────────────────
    // 极简优化测试
    // ──────────────────────────────────────────────
    
    #[test]
    fn test_optimize_minimal_basic() {
        // 测试基本的极简优化功能
        let xpath = "//Document[@AutomationId='RootWebArea' and @FrameworkId='Chrome']/Group[@FrameworkId='Chrome']";
        
        // 模拟验证回调：总是返回 true（简化测试）
        let verify_callback = |_xpath: &str| -> Result<bool> {
            Ok(true)
        };
        
        let progress_logs: std::sync::Arc<std::sync::Mutex<Vec<String>>> = 
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress_logs_clone = progress_logs.clone();
        
        let progress_callback = move |msg: &str| {
            progress_logs_clone.lock().unwrap().push(msg.to_string());
        };
        
        let cancel_flag = Arc::new(AtomicBool::new(false));
        
        let result = optimize_minimal_with_cancel(
            xpath,
            verify_callback,
            progress_callback,
            cancel_flag,
        ).unwrap();
        
        assert!(result.is_some(), "优化应该成功返回结果");
        let optimized = result.unwrap();
        
        println!("原始 XPath: {}", xpath);
        println!("优化后 XPath: {}", optimized);
        
        let logs = progress_logs.lock().unwrap();
        println!("日志条数: {}", logs.len());
        
        // 验证优化后的 XPath 更短
        assert!(optimized.len() <= xpath.len(), 
            "优化后的 XPath 应该更短或相等");
        
        // 验证有进度日志输出
        assert!(!logs.is_empty(), "应该有进度日志");
        
        // 验证包含关键日志信息
        let log_text = logs.join("\n");
        assert!(log_text.contains("开始") || log_text.contains("极简优化"), "应该包含开始日志");
        assert!(log_text.contains("优化完成") || log_text.contains("简化完成"), "应该包含完成日志");
    }
    
    #[test]
    fn test_optimize_minimal_with_cancellation() {
        // 测试取消功能
        let xpath = "//Document[@AutomationId='RootWebArea' and @FrameworkId='Chrome']";
        
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag_clone = cancel_flag.clone();
        
        // 使用 AtomicUsize 来计数尝试次数（线程安全）
        let attempt_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempt_count_clone = attempt_count.clone();
        
        // 在第一次尝试验证后设置取消标志
        let verify_callback = move |_xpath: &str| -> Result<bool> {
            attempt_count_clone.fetch_add(1, Ordering::SeqCst);
            if attempt_count_clone.load(Ordering::SeqCst) >= 1 {
                cancel_flag_clone.store(true, Ordering::SeqCst);
            }
            Ok(true)
        };
        
        let progress_callback = |_msg: &str| {};
        
        let result = optimize_minimal_with_cancel(
            xpath,
            verify_callback,
            progress_callback,
            cancel_flag,
        ).unwrap();
        
        // 取消后应该返回 None
        assert!(result.is_none(), "取消后应该返回 None");
    }
    
    #[test]
    fn test_optimize_minimal_selective_removal() {
        // 测试选择性移除：某些属性保留，某些移除
        let xpath = "//Document[@AutomationId='RootWebArea' and @FrameworkId='Chrome' and @LocalizedControlType='文档']";
        
        // 模拟验证：只保留 AutomationId 就能定位
        let verify_callback = |xpath: &str| -> Result<bool> {
            // 如果 XPath 包含 AutomationId，就认为可以定位
            Ok(xpath.contains("AutomationId='RootWebArea'"))
        };
        
        let progress_logs: std::sync::Arc<std::sync::Mutex<Vec<String>>> = 
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress_logs_clone = progress_logs.clone();
        
        let progress_callback = move |msg: &str| {
            progress_logs_clone.lock().unwrap().push(msg.to_string());
        };
        
        let cancel_flag = Arc::new(AtomicBool::new(false));
        
        let result = optimize_minimal_with_cancel(
            xpath,
            verify_callback,
            progress_callback,
            cancel_flag,
        ).unwrap();
        
        assert!(result.is_some(), "优化应该成功");
        let optimized = result.unwrap();
        
        println!("原始 XPath: {}", xpath);
        println!("优化后 XPath: {}", optimized);
        
        // 验证 AutomationId 被保留
        assert!(optimized.contains("AutomationId='RootWebArea'"),
            "AutomationId 应该被保留");
        
        // 验证 FrameworkId 和 LocalizedControlType 被移除
        assert!(!optimized.contains("@FrameworkId"),
            "FrameworkId 应该被移除");
        assert!(!optimized.contains("@LocalizedControlType"),
            "LocalizedControlType 应该被移除");
        
        // 验证 XPath 显著缩短
        let compression = (1.0 - optimized.len() as f64 / xpath.len() as f64) * 100.0;
        println!("压缩率: {:.1}%", compression);
        assert!(compression > 30.0, "压缩率应该大于 30%");
    }
    
    #[test]
    fn test_optimize_minimal_complex_xpath() {
        // 测试复杂的 XPath（类似用户提供的示例）
        let xpath = "//Document[@AutomationId='RootWebArea' and @FrameworkId='Chrome' and @LocalizedControlType='文档']/Group[@FrameworkId='Chrome' and @LocalizedControlType='组']/Group[starts-with(@ClassName, 'chat_mainPage__wilLn') and @FrameworkId='Chrome' and @LocalizedControlType='组']/Group[starts-with(@ClassName, 'temp-dialogue-btn_temp-dialogue') and @FrameworkId='Chrome' and @LocalizedControlType='组']";
        
        // 模拟验证：只要保留 AutomationId 和 ClassName 的前缀匹配即可
        let verify_callback = |xpath: &str| -> Result<bool> {
            let has_automation_id = xpath.contains("AutomationId='RootWebArea'");
            let has_class_prefix = xpath.contains("starts-with(@ClassName, 'chat_mainPage__wilLn')") ||
                                   xpath.contains("starts-with(@ClassName, 'temp-dialogue-btn_temp-dialogue')");
            Ok(has_automation_id || has_class_prefix)
        };
        
        let progress_logs: std::sync::Arc<std::sync::Mutex<Vec<String>>> = 
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress_logs_clone = progress_logs.clone();
        
        let progress_callback = move |msg: &str| {
            progress_logs_clone.lock().unwrap().push(msg.to_string());
        };
        
        let cancel_flag = Arc::new(AtomicBool::new(false));
        
        let result = optimize_minimal_with_cancel(
            xpath,
            verify_callback,
            progress_callback,
            cancel_flag,
        ).unwrap();
        
        assert!(result.is_some(), "复杂 XPath 优化应该成功");
        let optimized = result.unwrap();
        
        println!("原始 XPath 长度: {}", xpath.len());
        println!("优化后 XPath 长度: {}", optimized.len());
        
        let logs = progress_logs.lock().unwrap();
        println!("日志条数: {}", logs.len());
        
        // 显示前几条和后几条日志
        for (i, log) in logs.iter().enumerate() {
            if i < 5 || i >= logs.len() - 5 {
                println!("  [{}] {}", i, log);
            } else if i == 5 {
                println!("  ... ({} more logs)", logs.len() - 10);
            }
        }
        
        // 验证优化效果
        let compression = (1.0 - optimized.len() as f64 / xpath.len() as f64) * 100.0;
        println!("压缩率: {:.1}%", compression);
        
        // 验证 FrameworkId 和 LocalizedControlType 被大量移除
        let framework_count = optimized.matches("@FrameworkId").count();
        let localized_count = optimized.matches("@LocalizedControlType").count();
        println!("FrameworkId 出现次数: {}", framework_count);
        println!("LocalizedControlType 出现次数: {}", localized_count);
        
        // 原始 XPath 中有 4 个 FrameworkId 和 4 个 LocalizedControlType
        // 优化后应该显著减少
        assert!(framework_count < 4, "FrameworkId 应该被部分或全部移除");
        assert!(localized_count < 4, "LocalizedControlType 应该被部分或全部移除");
    }
    
    #[test]
    fn test_optimize_minimal_real_scenario() {
        // 【关键测试】使用用户真实捕获的 XPath
        let xpath = "//Document[@AutomationId='RootWebArea' and @FrameworkId='Chrome' and @LocalizedControlType='文档']/Group[@FrameworkId='Chrome' and @LocalizedControlType='组']/Group[starts-with(@ClassName, 'chat_mainPage__wilLn') and @FrameworkId='Chrome' and @LocalizedControlType='组']/Group[starts-with(@ClassName, 'temp-dialogue-btn_temp-dialogue') and @FrameworkId='Chrome' and @LocalizedControlType='组']";
        
        println!("\n=== 真实场景测试 ===");
        println!("原始 XPath: {}", xpath);
        println!("原始长度: {} 字符\n", xpath.len());
        
        // 先解析看看节点的实际属性
        let nodes = parse_xpath(xpath).unwrap();
        println!("解析后的节点：");
        for (i, node) in nodes.iter().enumerate() {
            println!("  [{}] {} - 属性数: {}", i, node.tag, node.attrs.len());
            for (k, v) in &node.attrs {
                println!("      @{} = '{}'", k, if v.len() > 60 { format!("{}...", &v[..60]) } else { v.clone() });
            }
        }
        println!();
        
        // 模拟真实验证逻辑（基于用户手动优化结果）：
        // - 如果 XPath 包含 AutomationId + **最后一个 Group** 有 starts-with(@ClassName, 'temp-dialogue-btn_temp-dialogue') + @FrameworkId='Chrome'
        //   且**中间节点不能有多余的 ClassName**（否则路径会太长） → 唯一
        // - 其他情况 → 不唯一或找不到
        let verify_callback = |test_xpath: &str| -> Result<bool> {
            let has_automation_id = test_xpath.contains("AutomationId='RootWebArea'");
            
            // 【关键】检查最后一个 Group 是否有目标 ClassName
            let parts: Vec<&str> = test_xpath.split('/').collect();
            let has_target_class_in_last_group = if let Some(last) = parts.last() {
                last.contains("starts-with(@ClassName, 'temp-dialogue-btn_temp-dialogue')")
            } else {
                false
            };
            
            // 检查最后一个 Group 是否有 FrameworkId
            let has_framework_in_last_group = if let Some(last) = parts.last() {
                last.contains("@FrameworkId='Chrome'")
            } else {
                false
            };
            
            // 【关键修复】检查中间节点（第2、3个 Group）是否有多余的 ClassName
            // 如果有，说明路径不够简洁，不应该认为是唯一的
            let has_extra_classname = parts.iter().enumerate().any(|(i, part)| {
                // 跳过第一个（Document）和最后一个（目标 Group）
                i > 0 && i < parts.len() - 1 && part.contains("@ClassName=")
            });
            
            // 模拟：需要 AutomationId + 最后一个 Group 的目标 ClassName + 最后一个 Group 的 FrameworkId
            //       且中间节点没有多余的 ClassName
            let is_unique = has_automation_id && has_target_class_in_last_group && has_framework_in_last_group && !has_extra_classname;
            
            if is_unique {
                println!("  [验证] ✓ 唯一匹配");
            } else {
                if has_extra_classname {
                    println!("  [验证] ✗ 中间节点有多余的 ClassName");
                } else {
                    println!("  [验证] ✗ 不唯一或缺少必要属性");
                }
            }
            
            Ok(is_unique)
        };
        
        let progress_logs: std::sync::Arc<std::sync::Mutex<Vec<String>>> = 
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress_logs_clone = progress_logs.clone();
        
        let progress_callback = move |msg: &str| {
            progress_logs_clone.lock().unwrap().push(msg.to_string());
            println!("{}", msg);  // 实时输出日志
        };
        
        let cancel_flag = Arc::new(AtomicBool::new(false));
        
        let result = optimize_minimal_with_cancel(
            xpath,
            verify_callback,
            progress_callback,
            cancel_flag,
        ).unwrap();
        
        assert!(result.is_some(), "优化应该成功返回结果");
        let optimized = result.unwrap();
        
        println!("\n=== 优化结果 ===");
        println!("优化后 XPath: {}", optimized);
        println!("优化后长度: {} 字符", optimized.len());
        
        let compression = (1.0 - optimized.len() as f64 / xpath.len() as f64) * 100.0;
        println!("压缩率: {:.1}%", compression);
        
        // 【关键验证】确保最终 XPath 能唯一定位
        let final_is_unique = verify_callback(&optimized).unwrap();
        assert!(final_is_unique, 
            "❌ 最终 XPath 必须能唯一定位！\nXPath: {}", optimized);
        
        // 验证保留了必要的属性
        assert!(optimized.contains("AutomationId='RootWebArea'"),
            "❌ AutomationId 是必需的，应该被保留");
        
        // 验证至少保留了一个 ClassName（因为需要它来确保唯一性）
        assert!(optimized.contains("@ClassName=") || optimized.contains("starts-with(@ClassName,"),
            "❌ 至少需要一个 ClassName 来确保唯一性");
        
        // 验证移除了冗余属性
        let framework_count = optimized.matches("@FrameworkId").count();
        let localized_count = optimized.matches("@LocalizedControlType").count();
        
        println!("\n=== 属性统计 ===");
        println!("@FrameworkId 出现次数: {} (原始: 4次)", framework_count);
        println!("@LocalizedControlType 出现次数: {} (原始: 4次)", localized_count);
        
        // FrameworkId 和 LocalizedControlType 应该被大量移除
        assert!(framework_count < 4, "FrameworkId 应该被部分或全部移除");
        assert!(localized_count < 4, "LocalizedControlType 应该被部分或全部移除");
        
        println!("\n✅ 测试通过！极简优化算法正确工作。");
    }
    
    #[test]
    fn test_optimize_minimal_preserves_essential_attrs() {
        // 测试保留必要属性：AutomationId 和唯一的 ClassName
        let xpath = "//Button[@AutomationId='submit' and @Name='提交' and @FrameworkId='Chrome']";
        
        // 模拟验证：必须保留 AutomationId
        let verify_callback = |xpath: &str| -> Result<bool> {
            Ok(xpath.contains("AutomationId='submit'"))
        };
        
        let progress_callback = |_msg: &str| {};
        let cancel_flag = Arc::new(AtomicBool::new(false));
        
        let result = optimize_minimal_with_cancel(
            xpath,
            verify_callback,
            progress_callback,
            cancel_flag,
        ).unwrap();
        
        assert!(result.is_some());
        let optimized = result.unwrap();
        
        println!("原始: {}", xpath);
        println!("优化: {}", optimized);
        
        // 验证 AutomationId 被保留
        assert!(optimized.contains("AutomationId='submit'"),
            "AutomationId 是必需的，应该被保留");
        
        // 验证 Name 可能被移除（因为不是必需的）
        // 注意：这取决于优化算法的尝试顺序
    }
}
