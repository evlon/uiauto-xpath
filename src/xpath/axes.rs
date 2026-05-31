use super::ast::Axis;
use crate::element::UiElement;
use crate::error::Result;

/// Select elements along the given axis from a context node.
///
/// Uses RawViewWalker by default for Child/Descendant/DescendantOrSelf axes
/// to ensure consistency with capture-side tree traversal. The control view
/// (ControlViewWalker) filters out intermediate elements in some frameworks
/// (e.g., Qt/WeChat), which causes XPath lookups to fail for elements that
/// were captured via RawViewWalker.
pub fn select_axis(node: &UiElement, axis: Axis) -> Result<Vec<UiElement>> {
    Ok(match axis {
        Axis::Self_ => vec![node.clone()],
        Axis::Child => node.raw_children()?,
        Axis::Parent => node.parent().into_iter().collect(),
        Axis::Descendant => node.raw_descendants()?,
        Axis::DescendantOrSelf => {
            let mut v = vec![node.clone()];
            v.extend(node.raw_descendants()?);
            v
        }
        Axis::Ancestor => node.ancestors(),
        Axis::AncestorOrSelf => {
            let mut v = vec![node.clone()];
            v.extend(node.ancestors());
            v
        }
        Axis::FollowingSibling => node.following_siblings()?,
        Axis::PrecedingSibling => node.preceding_siblings()?,
        Axis::Following => {
            let mut out = Vec::new();
            let mut cur = Some(node.clone());
            while let Some(c) = cur {
                for s in c.following_siblings()? {
                    out.push(s.clone());
                    out.extend(s.raw_descendants()?);
                }
                cur = c.parent();
            }
            out
        }
        Axis::Preceding => {
            let mut out = Vec::new();
            let mut cur = Some(node.clone());
            while let Some(c) = cur {
                for s in c.preceding_siblings()? {
                    out.extend(s.raw_descendants()?);
                    out.push(s);
                }
                cur = c.parent();
            }
            out
        }
        Axis::Attribute | Axis::Namespace => Vec::new(),
    })
}

pub fn is_reverse_axis(axis: Axis) -> bool {
    matches!(axis,
        Axis::Parent | Axis::Ancestor | Axis::AncestorOrSelf |
        Axis::PrecedingSibling | Axis::Preceding)
}
