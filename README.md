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

## 依赖

- Windows 10/11
- Rust 2021 Edition

## License

MIT