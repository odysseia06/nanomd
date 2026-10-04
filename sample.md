# nano.md sample

A tiny, fast markdown viewer/editor. Press **Ctrl+E** (or the toolbar button) to
see this file's raw source; press it again to come back.

## Formatting

**Bold**, *italic*, ~~strikethrough~~, `inline code`, and a [link](https://example.com).

> Blockquotes render indented, like this.

## Lists

1. Ordered item
2. Another item
   - Nested bullet
   - Another bullet

- [x] Task list: done item
- [ ] Task list: open item

## Table

| Feature        | Status |
|----------------|--------|
| Rendered view  | ✔      |
| Raw editing    | ✔      |
| One-click swap | ✔      |

## Code

```rust
fn main() {
    println!("hello from nano.md");
}
```

## Math

Inline math: $E = mc^2$, $a^2 + b^2 = c^2$, and $\alpha + \beta = \gamma$.

$$
x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}
$$

$$
\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}
$$

$$
\begin{pmatrix} a & b \\ c & d \end{pmatrix}
$$

Code stays literal: `$x^2$`. So do escaped prices: \$5 and \$10.

## Images

Remote images become plain links (no network access): ![remote logo](https://example.com/logo.png)

Local images render inline, resolved relative to this file: ![nano.md icon](assets/icon-512.png)

---

*That's the whole tour — footnotes work too.[^1]*

[^1]: Like this one.
