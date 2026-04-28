use super::ast::Axis;
use crate::element::UiElement;
use crate::error::Result;

pub fn select_axis(node: &UiElement, axis: Axis) -> Result<Vec<UiElement>> {
    Ok(match axis {
        Axis::Self_ => vec![node.clone()],
        Axis::Child => node.children()?,
        Axis::Parent => node.parent().into_iter().collect(),
        Axis::Descendant => node.descendants()?,
        Axis::DescendantOrSelf => {
            let mut v = vec![node.clone()];
            v.extend(node.descendants()?);
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
            // 所有文档顺序之后且不在祖先链上的节点
            let mut out = Vec::new();
            let mut cur = Some(node.clone());
            while let Some(c) = cur {
                for s in c.following_siblings()? {
                    out.push(s.clone());
                    out.extend(s.descendants()?);
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
                    out.extend(s.descendants()?);
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
