use crate::app::Message;
use notify::{EventKind, RecursiveMode, Watcher};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::path::Path;
use std::sync::mpsc::Sender;

/// Forward terminal input to the channel from a background thread.
pub fn spawn_input(tx: Sender<Message>) {
  std::thread::spawn(move || {
    loop {
      let msg = match event::read() {
        Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => Message::Key(k),
        Ok(Event::Resize(_, _)) => Message::Resize,
        Ok(_) => continue,
        Err(_) => break,
      };
      if tx.send(msg).is_err() {
        break;
      }
    }
  });
}

/// Watch the journal's directory (renames replace the file's inode) and report changes to it.
pub fn watch(path: &Path, tx: Sender<Message>) -> notify::Result<notify::RecommendedWatcher> {
  let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
  let name = target.file_name().map(|n| n.to_os_string());
  let dir = crate::ledger::writer::parent_dir(&target);
  let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
    let Ok(ev) = res else { return };
    if matches!(ev.kind, EventKind::Access(_)) {
      return;
    }
    if ev
      .paths
      .iter()
      .any(|p| p.file_name().map(|n| n.to_os_string()) == name)
    {
      let _ = tx.send(Message::FileChanged);
    }
  })?;
  watcher.watch(&dir, RecursiveMode::NonRecursive)?;
  Ok(watcher)
}
