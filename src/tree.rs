use serde_json::Value;

#[derive(Debug, Clone)]
pub struct WinInfo {
    pub id: i64,
    pub fs: i64,
    pub marks: Vec<String>,
    pub focused: bool,
    pub border: String,
    pub border_width: i64,
}

impl WinInfo {
    pub fn has_auto_fs(&self) -> bool {
        self.marks.iter().any(|m| m == "_auto_fs")
    }

    pub fn is_fs(&self) -> bool {
        self.fs > 0
    }
}

#[derive(Debug)]
pub struct Snapshot {
    pub ws_name: String,
    pub ws_layout: String,
    pub tiled: Vec<WinInfo>,
    pub float_n: usize,
    pub float_fs: usize,
    pub any_focused: bool,
    pub global_focused: Option<i64>,
}

pub fn parse_win(v: &Value) -> Option<WinInfo> {
    let pid = v.get("pid")?.as_i64()?;
    if pid <= 0 {
        return None;
    }
    Some(WinInfo {
        id: v.get("id")?.as_i64()?,
        fs: v
            .get("fullscreen_mode")
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
        marks: v
            .get("marks")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        focused: v.get("focused").and_then(|x| x.as_bool()).unwrap_or(false),
        border: v
            .get("border")
            .and_then(|x| x.as_str())
            .unwrap_or("normal")
            .to_owned(),
        border_width: v
            .get("current_border_width")
            .and_then(|x| x.as_i64())
            .unwrap_or(2),
    })
}

pub fn collect_tiled(node: &Value, out: &mut Vec<WinInfo>) {
    if let Some(w) = parse_win(node) {
        out.push(w);
        return;
    }
    for child in node
        .get("nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        collect_tiled(child, out);
    }
}

pub fn count_floating(ws: &Value) -> (usize, usize) {
    fn walk(v: &Value, normal: &mut usize, fullscreen: &mut usize) {
        if let Some(pid) = v.get("pid").and_then(|x| x.as_i64()) {
            if pid > 0 {
                if v.get("fullscreen_mode")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0)
                    > 0
                {
                    *fullscreen += 1;
                } else {
                    *normal += 1;
                }
                return;
            }
        }
        for c in v
            .get("nodes")
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
        {
            walk(c, normal, fullscreen);
        }
    }
    let (mut n, mut fs) = (0, 0);
    for fnode in ws
        .get("floating_nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        walk(fnode, &mut n, &mut fs);
    }
    (n, fs)
}

pub fn has_focused_descendant(node: &Value) -> bool {
    if node.get("focused").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    for child in node
        .get("nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        if has_focused_descendant(child) {
            return true;
        }
    }
    for child in node
        .get("floating_nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        if has_focused_descendant(child) {
            return true;
        }
    }
    false
}

pub fn find_focused_window(node: &Value) -> Option<i64> {
    if let Some(pid) = node.get("pid").and_then(|v| v.as_i64()) {
        if pid > 0 {
            if node.get("focused").and_then(|v| v.as_bool()) == Some(true) {
                return node.get("id").and_then(|v| v.as_i64());
            }
            return None;
        }
    }
    for child in node
        .get("nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(id) = find_focused_window(child) {
            return Some(id);
        }
    }
    for child in node
        .get("floating_nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(id) = find_focused_window(child) {
            return Some(id);
        }
    }
    None
}

pub fn contains_con_id(node: &Value, target_id: i64) -> bool {
    if node.get("id").and_then(|v| v.as_i64()) == Some(target_id) {
        return true;
    }
    for child in node
        .get("nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        if contains_con_id(child, target_id) {
            return true;
        }
    }
    for child in node
        .get("floating_nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        if contains_con_id(child, target_id) {
            return true;
        }
    }
    false
}
