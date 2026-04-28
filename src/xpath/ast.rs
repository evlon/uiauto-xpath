#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Child, Descendant, DescendantOrSelf,
    Parent, Ancestor, AncestorOrSelf,
    Self_, Attribute, Namespace,
    FollowingSibling, PrecedingSibling,
    Following, Preceding,
}

#[derive(Debug, Clone)]
pub enum NodeTest {
    /// * 或 name
    Name(String),
    Wildcard,
    /// node()
    Node,
    /// text()
    Text,
    /// comment()
    Comment,
    /// processing-instruction([literal])
    ProcessingInstruction(Option<String>),
}

#[derive(Debug, Clone)]
pub struct Step {
    pub axis: Axis,
    pub test: NodeTest,
    pub predicates: Vec<Expr>,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    String(String),
    VarRef(String),
    Path(PathExpr),
    Filter { primary: Box<Expr>, predicates: Vec<Expr>, steps: Vec<Step> },
    FunctionCall { name: String, args: Vec<Expr> },
    BinaryOp(BinOp, Box<Expr>, Box<Expr>),
    UnaryMinus(Box<Expr>),
    Union(Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or, And,
    Eq, NotEq, Lt, Le, Gt, Ge,
    Add, Sub, Mul, Div, Mod,
}

#[derive(Debug, Clone)]
pub struct PathExpr {
    pub absolute: bool,
    /// 当 absolute 且使用 // 起始
    pub leading_descendant: bool,
    pub steps: Vec<Step>,
}
