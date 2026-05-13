# uiauto-xpath

基于 Windows UI Automation 的 XPath 查询库，用于在 Windows UI 元素树中执行 XPath 查询。

## 使用示例

```rust
use uiauto_xpath::{UiAutomation, XPath};

fn main() -> uiauto_xpath::Result<()> {
    let uia = UiAutomation::new()?;
    let root = uia.root()?;

    // 查找所有按钮
    let xpath = XPath::compile("//Button")?;
    for btn in xpath.select_nodes(&root)? {
        println!("Button: {:?}", btn);
    }

    // 复杂查询：包含 "OK" 文本且启用的按钮
    let xpath = XPath::compile(
        "//Window[contains(@Name,'记事本')]//Button[@IsEnabled='true' and starts-with(@Name,'确')]"
    )?;
    if let Some(b) = xpath.select_first(&root)? {
        println!("找到按钮: {}", b.name());
    }

    // 使用轴和位置谓词
    let xpath = XPath::compile("/descendant::Edit[position()=1]")?;
    let _ = xpath.select_nodes(&root)?;

    // 字符串函数
    let xpath = XPath::compile("//*[matches(@Name, '^Save.*')]")?;
    let _ = xpath.select_nodes(&root)?;

    Ok(())
}
```

## 功能清单

### XPath 标准支持

| 类别 | 支持项 |
|------|--------|
| **轴 (13种)** | `child`、`descendant`、`parent`、`ancestor`、`self`、`descendant-or-self`、`ancestor-or-self`、`following`、`preceding`、`following-sibling`、`preceding-sibling`、`attribute`、`namespace` |
| **简写语法** | `/`、`//`、`.`、`..`、`@`、`*` |
| **节点测试** | `name`、`*`、`node()`、`text()`、`comment()`、`processing-instruction()` |
| **谓词** | `[...]`，支持位置、布尔、数值条件 |
| **运算符** | `=`、`!=`、`<`、`<=`、`>`、`>=`、`+`、`-`、`*`、`div`、`mod`、`and`、`or`、`|` |
| **字面量** | 数字、字符串、变量 `$var` |

### 内置函数

| 类别 | 函数 |
|------|------|
| **节点集** | `last`、`position`、`count`、`id`、`local-name`、`name`、`namespace-uri` |
| **字符串** | `string`、`concat`、`starts-with`、`ends-with`、`contains`、`substring-before`、`substring-after`、`substring`、`string-length`、`normalize-space`、`translate`、`upper-case`、`lower-case`、`matches`、`replace` |
| **布尔** | `boolean`、`not`、`true`、`false`、`lang` |
| **数值** | `number`、`sum`、`floor`、`ceiling`、`round`、`abs` |

### UIA 适配

- **元素属性映射为 XPath 属性**：
  - `@Name`、`@ClassName`、`@AutomationId`、`@ControlType`
  - `@IsEnabled`、`@IsOffscreen`、`@ProcessId`、`@HelpText`、`@FrameworkId`
- **控件类型作为节点名**：如 `Button`、`Edit`、`Window`

## API

### UiAutomation

```rust
// 创建 UI Automation 实例
let uia = UiAutomation::new()?;

// 获取根元素（桌面）
let root = uia.root()?;

// 从窗口句柄获取元素
let elem = uia.from_handle(hwnd)?;
```

### UiElement

```rust
// 获取元素属性
elem.name();           // 名称
elem.class_name();     // 类名
elem.automation_id();  // AutomationId
elem.control_type_name(); // 控件类型名（如 "Button"）
elem.is_enabled();     // 是否启用
elem.process_id();     // 进程ID

// 遍历元素树
elem.children();       // 子元素
elem.parent();         // 父元素
elem.descendants();    // 所有后代
elem.ancestors();      // 所有祖先
elem.following_siblings();  // 后续兄弟
elem.preceding_siblings();  // 前置兄弟

// 比较元素
elem.equals(&other);   // 是否同一元素
```

### XPath

```rust
// 编译 XPath 表达式
let xpath = XPath::compile("//Button[@Name='OK']")?;

// 执行查询
let nodes = xpath.select_nodes(&root)?;    // 返回所有匹配节点
let first = xpath.select_first(&root)?;    // 返回第一个匹配节点（Option）
let result = xpath.evaluate(&root)?;       // 返回任意类型结果
```

## XPath 智能优化器（实测示例）

优化器将冗长、脆弱的 XPath 压缩为简洁、稳定的表达式。

### 输入 → 输出（实测）

**输入**（14 层嵌套，~900 字符）：
```
//Pane[@ClassName='ChromeWidgetWin1' ...]/Pane.../Document[@AutomationId='RootWebArea' ...]/Group/.../Group[@ClassName='temp-dialogue-btntemp-dialogue-btnBOp4 ...']
```

**锚点选择**：`Document[11]`（得分 18，含 `AutomationId='RootWebArea'`）

**anchor_relative（主推荐）**：
```xpath
//Document[@AutomationId='RootWebArea']/Group/Group/Group[starts-with(@ClassName, 'temp-dialogue-btntemp-dialogue-btn')]
```

**minimal（备选）**：
```xpath
//Document[@AutomationId='RootWebArea']//Group[starts-with(@ClassName, 'temp-dialogue-btntemp-dialogue-btn')]
```

**压缩率**：92%

### 三个核心算法

#### 1. 动态 ClassName 检测 `is_dynamic_class`

判断标准：
- 超过 2 个空格分隔的 token
- 或任意 token 中的 camelCase 词含有随机特征（纯大写词、大写+数字混合词）

**识别示例**：`chatmainPagewilLn`、`btnBOp4`、`ZJ07f` 都能正确识别为动态类名。

#### 2. 稳定前缀提取 `extract_stable_prefix`

算法流程：
1. 先按 `-` 分割
2. 再对每段做 camelCase 词边界拆分
3. 找到第一个随机词就截断

**示例**：
```
temp-dialogue-btnBOp4
→ 拆出 ["temp", "dialogue", "btn|BOp4"]
→ btn 的 camel 词中 BOp4 是随机词
→ 稳定前缀: temp-dialogue-btn
```

#### 3. 锚点评分 `anchor_score`

评分规则：
- **AutomationId**：权重最高（10 分）
- **语义化标签**：`Document`/`Edit`/`Button` 等额外加分
- **容器标签**：`Pane`/`Group` 得 0 分
- 保证选出语义最强的节点作为跳转起点

## 依赖

- Windows 10/11
- Rust 2021 Edition

## 调试日志

默认情况下，库只输出 `info` 级别及以上的日志。如果需要查看详细的 XPath 优化和 UIA Condition 构建过程，可以设置环境变量：

```bash
# 查看详细调试日志
set RUST_LOG=uiauto_xpath=debug
cargo run --bin element-selector

# 或者在 Linux/macOS 上
export RUST_LOG=uiauto_xpath=debug
cargo run --bin element-selector
```

调试日志包括：
- XPath 步骤优化决策（是否使用 FindAll）
- UIA Condition 构建过程（属性映射、条件合并）
- FindAll 返回的元素数量
- 复杂谓词过滤结果

## License

MIT