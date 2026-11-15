//! An ordered JSON value, written the way Python's `json.dumps` writes it, so
//! the kubeconfig keeps the key order and layout the Python tool gave it.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum J {
    S(String),
    B(bool),
    A(Vec<J>),
    O(Vec<(String, J)>),
}

fn string(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

impl J {
    /// `indent=2`, no trailing newline.
    pub fn pretty(&self) -> String {
        self.write(Some(0))
    }

    /// One line, `", "` and `": "` separators.
    pub fn compact(&self) -> String {
        self.write(None)
    }

    fn write(&self, depth: Option<usize>) -> String {
        match self {
            J::S(text) => string(text),
            J::B(value) => value.to_string(),
            J::A(items) => {
                let parts = items
                    .iter()
                    .map(|i| i.write(depth.map(|d| d + 1)))
                    .collect();
                wrap('[', ']', parts, depth)
            }
            J::O(fields) => {
                let parts = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", string(k), v.write(depth.map(|d| d + 1))))
                    .collect();
                wrap('{', '}', parts, depth)
            }
        }
    }
}

fn wrap(open: char, close: char, parts: Vec<String>, depth: Option<usize>) -> String {
    if parts.is_empty() {
        return format!("{open}{close}");
    }
    match depth {
        None => format!("{open}{}{close}", parts.join(", ")),
        Some(d) => {
            let inner = "  ".repeat(d + 1);
            let outer = "  ".repeat(d);
            let body: Vec<String> = parts.iter().map(|p| format!("{inner}{p}")).collect();
            format!("{open}\n{}\n{outer}{close}", body.join(",\n"))
        }
    }
}
