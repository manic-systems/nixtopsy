use std::{
   io,
   mem,
   time::{
      Duration,
      Instant,
   },
};

use crossterm::{
   event::{
      self,
      Event,
      KeyCode,
      KeyEvent,
      KeyEventKind,
      KeyModifiers,
   },
   execute,
   terminal::{
      EnterAlternateScreen,
      LeaveAlternateScreen,
      disable_raw_mode,
      enable_raw_mode,
   },
};
use ratatui::{
   Frame,
   Terminal,
   backend::CrosstermBackend,
   layout::{
      Constraint,
      Direction,
      Layout,
      Rect,
   },
   style::{
      Color,
      Modifier,
      Style,
   },
   text::{
      Line,
      Span,
   },
   widgets::{
      Block,
      Borders,
      Clear,
      List,
      ListItem,
      ListState,
      Paragraph,
      Wrap,
   },
};

use crate::{
   clipboard,
   hrfsize::{
      SizeUnits,
      pretty_size,
   },
   index::InvertedIndex,
   stats::{
      PathStats,
      fmt_immediate_parents,
      shortest_path_to,
      why_depends,
   },
   store::{
      NodeId,
      StoreEnv,
      StorePath,
   },
};

const HIGHLIGHT_DURATION: Duration = Duration::from_millis(500);
const YANK_FEEDBACK_DURATION: Duration = Duration::from_millis(1500);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
   Alphabetical,
   ClosureSize,
   AddedSize,
}

impl SortOrder {
   const fn next(self) -> Self {
      match self {
         Self::Alphabetical => Self::ClosureSize,
         Self::ClosureSize => Self::AddedSize,
         Self::AddedSize => Self::Alphabetical,
      }
   }
}

#[derive(Debug)]
struct Pane {
   items: Vec<NodeId>,
   state: ListState,
}

impl Pane {
   fn empty() -> Self {
      Self {
         items: Vec::new(),
         state: ListState::default(),
      }
   }

   fn from_items(items: Vec<NodeId>, selected: Option<usize>) -> Self {
      let mut state = ListState::default();
      if !items.is_empty() {
         state.select(Some(selected.unwrap_or(0).min(items.len() - 1)));
      }
      Self { items, state }
   }

   fn selected(&self) -> Option<NodeId> {
      let i = self.state.selected()?;
      self.items.get(i).copied()
   }

   fn move_down(&mut self) {
      if self.items.is_empty() {
         return;
      }
      let new = self
         .state
         .selected()
         .map_or(0, |i| (i + 1).min(self.items.len() - 1));
      self.state.select(Some(new));
   }

   fn move_up(&mut self) {
      if self.items.is_empty() {
         return;
      }
      let new = self.state.selected().map_or(0, |i| i.saturating_sub(1));
      self.state.select(Some(new));
   }

   fn move_page_down(&mut self, page: usize) {
      if self.items.is_empty() {
         return;
      }
      let new = self
         .state
         .selected()
         .map_or(0, |i| (i + page.max(1)).min(self.items.len() - 1));
      self.state.select(Some(new));
   }

   fn move_page_up(&mut self, page: usize) {
      if self.items.is_empty() {
         return;
      }
      let new = self
         .state
         .selected()
         .map_or(0, |i| i.saturating_sub(page.max(1)));
      self.state.select(Some(new));
   }
}

enum Modal {
   Notice {
      title: String,
      body:  String,
   },
   WhyDepends {
      paths:    Vec<Vec<NodeId>>,
      state:    ListState,
      h_scroll: u16,
   },
   Search {
      left:    String,
      right:   String,
      results: Vec<NodeId>,
      state:   ListState,
   },
}

pub struct App {
   env:             StoreEnv<PathStats>,
   index:           InvertedIndex<NodeId>,
   prev:            Pane,
   curr:            Pane,
   next:            Pane,
   parents:         Vec<Pane>,
   modal:           Option<Modal>,
   sort_order:      SortOrder,
   sort_changed_at: Option<Instant>,
   yanked:          Option<(Instant, String)>,
   last_page_size:  usize,
   units:           SizeUnits,
}

impl App {
   pub fn new(env: StoreEnv<PathStats>, units: SizeUnits) -> Self {
      let mut idx = InvertedIndex::<NodeId>::new();
      for (id, sp) in env.paths.iter().enumerate() {
         idx.insert(sp.name.name.clone(), id as NodeId);
      }

      let default_sort = SortOrder::ClosureSize;
      let sorted_roots = sort_ids(&env, &env.roots, default_sort);

      let curr = Pane::from_items(sorted_roots, Some(0));
      let mut app = Self {
         env,
         index: idx,
         prev: Pane::empty(),
         curr,
         next: Pane::empty(),
         parents: Vec::new(),
         modal: None,
         sort_order: default_sort,
         sort_changed_at: None,
         yanked: None,
         last_page_size: 10,
         units,
      };
      app.repopulate_next();
      app
   }

   fn selected_path(&self) -> Option<&StorePath<PathStats>> {
      self.curr.selected().map(|id| self.env.get(id))
   }

   fn selected_id(&self) -> Option<NodeId> {
      self.curr.selected()
   }

   fn selected_route(&self) -> Vec<NodeId> {
      let mut out = Vec::new();
      for p in &self.parents {
         if let Some(id) = p.selected() {
            out.push(id);
         }
      }
      if let Some(id) = self.prev.selected() {
         out.push(id);
      }
      if let Some(id) = self.curr.selected() {
         out.push(id);
      }
      out
   }

   fn repopulate_next(&mut self) {
      let Some(id) = self.curr.selected() else {
         self.next = Pane::empty();
         return;
      };
      let sorted = sort_ids(&self.env, &self.env.get(id).refs, self.sort_order);
      self.next = Pane::from_items(sorted, Some(0));
   }

   fn move_right(&mut self) {
      if self.next.items.is_empty() {
         return;
      }
      let new_curr = mem::replace(&mut self.next, Pane::empty());
      let new_prev = mem::replace(&mut self.curr, new_curr);
      let old_prev = mem::replace(&mut self.prev, new_prev);
      self.parents.push(old_prev);
      self.repopulate_next();
   }

   fn move_left(&mut self) {
      let Some(parent) = self.parents.pop() else {
         return;
      };
      let new_next = mem::replace(&mut self.curr, mem::replace(&mut self.prev, parent));
      self.next = new_next;
   }

   fn cycle_sort(&mut self) {
      self.sort_order = self.sort_order.next();
      self.sort_changed_at = Some(Instant::now());
      let so = self.sort_order;
      sort_pane_in_place(&self.env, &mut self.curr, so);
      sort_pane_in_place(&self.env, &mut self.next, so);
      sort_pane_in_place(&self.env, &mut self.prev, so);
      for p in &mut self.parents {
         sort_pane_in_place(&self.env, p, so);
      }
   }

   fn highlight_active(&self) -> bool {
      self
         .sort_changed_at
         .is_some_and(|t| t.elapsed() < HIGHLIGHT_DURATION)
   }

   fn yank_message(&self) -> Option<&str> {
      let (t, path) = self.yanked.as_ref()?;
      (t.elapsed() < YANK_FEEDBACK_DURATION).then_some(path.as_str())
   }

   fn yank_selected(&mut self) {
      let Some(sp) = self.selected_path() else {
         return;
      };
      let path = sp.name.to_path(&self.env.store);
      match clipboard::copy(&path) {
         Ok(()) => {
            self.yanked = Some((Instant::now(), path));
         },
         Err(errs) => {
            let mut body = String::from("Cannot copy to clipboard:\n");
            for e in &errs {
               body.push_str("  ");
               body.push_str(e);
               body.push('\n');
            }
            body.push_str("Please report this as a bug.");
            self.modal = Some(Modal::Notice {
               title: "Error".to_string(),
               body,
            });
         },
      }
   }

   fn open_help(&mut self) {
      self.modal = Some(Modal::Notice {
         title: "Help".to_string(),
         body:  help_text().to_string(),
      });
   }

   fn open_why_depends(&mut self) {
      let Some(selected_id) = self.selected_id() else {
         return;
      };
      let paths = why_depends(&self.env, selected_id);
      if paths.is_empty() {
         return;
      }
      let route = self.selected_route();
      let mut state = ListState::default();
      let initial = paths.iter().position(|p| p == &route).unwrap_or(0);
      state.select(Some(initial.min(paths.len() - 1)));
      self.modal = Some(Modal::WhyDepends {
         paths,
         state,
         h_scroll: 0,
      });
   }

   fn open_search(&mut self) {
      let results = run_search(&self.index, "");
      let mut state = ListState::default();
      if !results.is_empty() {
         state.select(Some(0));
      }
      self.modal = Some(Modal::Search {
         left: String::new(),
         right: String::new(),
         results,
         state,
      });
   }

   fn select_route(&mut self, route: Vec<NodeId>) {
      if route.is_empty() {
         return;
      }
      let so = self.sort_order;

      let sorted = sort_ids(&self.env, &self.env.roots, so);
      let sel = sorted.iter().position(|&id| id == route[0]);
      let mut panes = Vec::<Pane>::with_capacity(route.len());
      panes.push(Pane::from_items(sorted, sel));

      for window in route.windows(2) {
         let (parent, child) = (window[0], window[1]);
         let sorted = sort_ids(&self.env, &self.env.get(parent).refs, so);
         let sel = sorted.iter().position(|&id| id == child);
         panes.push(Pane::from_items(sorted, sel));
      }

      // Panes are root-first. Last pane = curr; second-to-last = prev; rest =
      // parents. `move_right` pushes the previous (empty) `prev` onto `parents`
      // before replacing it with what was `curr`, so the parents stack always
      // has one empty sentinel below the root pane. We preserve that here so
      // `move_left` can pop back to the root pane.
      let curr = panes.pop().unwrap_or_else(Pane::empty);
      let prev = panes.pop().unwrap_or_else(Pane::empty);
      let mut parents = panes;
      if !parents.is_empty() || !prev.items.is_empty() {
         parents.insert(0, Pane::empty());
      }

      self.parents = parents;
      self.prev = prev;
      self.curr = curr;
      self.repopulate_next();
   }
}

fn compare_ids(
   env: &StoreEnv<PathStats>,
   a: NodeId,
   b: NodeId,
   so: SortOrder,
) -> std::cmp::Ordering {
   let ap = env.get(a);
   let bp = env.get(b);
   match so {
      SortOrder::Alphabetical => {
         ap.name
            .short()
            .to_lowercase()
            .cmp(&bp.name.short().to_lowercase())
      },
      SortOrder::ClosureSize => bp.payload.total_size.cmp(&ap.payload.total_size),
      SortOrder::AddedSize => bp.payload.added_size.cmp(&ap.payload.added_size),
   }
}

fn sort_ids(env: &StoreEnv<PathStats>, ids: &[NodeId], so: SortOrder) -> Vec<NodeId> {
   let mut v = ids.to_vec();
   v.sort_by(|&a, &b| compare_ids(env, a, b, so));
   v
}

fn sort_pane_in_place(env: &StoreEnv<PathStats>, pane: &mut Pane, so: SortOrder) {
   let selected_id = pane.selected();
   pane.items.sort_by(|&a, &b| compare_ids(env, a, b, so));
   let new_sel = selected_id
      .and_then(|s| pane.items.iter().position(|&id| id == s))
      .or(if pane.items.is_empty() { None } else { Some(0) });
   pane.state.select(new_sel);
}

fn run_search(index: &InvertedIndex<NodeId>, needle: &str) -> Vec<NodeId> {
   index.search(needle).into_values().collect()
}

pub const fn help_text() -> &'static str {
   "hjkl/Arrow Keys : Navigate
w               : Open why-depends modal
/               : Open search modal
s               : Change sort order
y               : Yank selected path to clipboard
?               : Show help
q/Esc           : Quit / close modal"
}

pub fn run(env: StoreEnv<PathStats>, units: SizeUnits) -> io::Result<()> {
   enable_raw_mode()?;
   let mut stdout = io::stdout();
   execute!(stdout, EnterAlternateScreen)?;
   let backend = CrosstermBackend::new(stdout);
   let mut terminal = Terminal::new(backend)?;

   let result = run_app(&mut terminal, env, units);

   disable_raw_mode()?;
   execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
   terminal.show_cursor()?;
   result
}

fn run_app(
   terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
   env: StoreEnv<PathStats>,
   units: SizeUnits,
) -> io::Result<()> {
   let mut app = App::new(env, units);

   loop {
      terminal
         .draw(|f| draw(f, &mut app))
         .map_err(|e| io::Error::other(format!("draw error: {e}")))?;
      if event::poll(POLL_INTERVAL)? {
         match event::read()? {
            Event::Key(k)
               if (k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat)
                  && handle_key(&mut app, k) =>
            {
               break;
            },
            _ => {},
         }
      }
   }
   Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) -> bool {
   if app.modal.is_some() {
      return handle_modal_key(app, key);
   }

   match key.code {
      KeyCode::Char('q') | KeyCode::Esc => return true,
      KeyCode::Char('?') => app.open_help(),
      KeyCode::Char('w') => app.open_why_depends(),
      KeyCode::Char('/') => app.open_search(),
      KeyCode::Char('y') => app.yank_selected(),
      KeyCode::Char('s') => app.cycle_sort(),
      KeyCode::Char('h') | KeyCode::Left => app.move_left(),
      KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
         app.curr.move_down();
         app.repopulate_next();
      },
      KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
         app.curr.move_up();
         app.repopulate_next();
      },
      KeyCode::Char('l') | KeyCode::Right => app.move_right(),
      KeyCode::PageDown => {
         let p = app.last_page_size;
         app.curr.move_page_down(p);
         app.repopulate_next();
      },
      KeyCode::PageUp => {
         let p = app.last_page_size;
         app.curr.move_page_up(p);
         app.repopulate_next();
      },
      KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => return true,
      _ => {},
   }
   false
}

fn handle_modal_key(app: &mut App, key: KeyEvent) -> bool {
   let Some(modal) = app.modal.take() else {
      return false;
   };
   match modal {
      Modal::Notice { title, body } => {
         match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {},
            _ => app.modal = Some(Modal::Notice { title, body }),
         }
      },
      Modal::WhyDepends {
         paths,
         mut state,
         mut h_scroll,
      } => {
         match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {},
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
               move_list_down(&mut state, paths.len());
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
            KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
               move_list_up(&mut state);
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
            KeyCode::Char('h') | KeyCode::Left => {
               h_scroll = h_scroll.saturating_sub(1);
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
            KeyCode::Char('l') | KeyCode::Right => {
               h_scroll = h_scroll.saturating_add(1);
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
            KeyCode::Home => {
               h_scroll = 0;
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
            KeyCode::End => {
               if let Some(idx) = state.selected()
                  && let Some(p) = paths.get(idx)
               {
                  h_scroll = path_display_len(&app.env, p) as u16;
               }
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
            KeyCode::PageDown => {
               for _ in 0..10 {
                  move_list_down(&mut state, paths.len());
               }
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
            KeyCode::PageUp => {
               for _ in 0..10 {
                  move_list_up(&mut state);
               }
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
            KeyCode::Enter => {
               if let Some(idx) = state.selected()
                  && let Some(path) = paths.get(idx).cloned()
               {
                  app.select_route(path);
               }
            },
            _ => {
               app.modal = Some(Modal::WhyDepends {
                  paths,
                  state,
                  h_scroll,
               });
            },
         }
      },
      Modal::Search {
         mut left,
         mut right,
         mut results,
         mut state,
      } => {
         match key.code {
            KeyCode::Esc => {},
            KeyCode::Down | KeyCode::Tab => {
               move_list_down(&mut state, results.len());
               app.modal = Some(Modal::Search {
                  left,
                  right,
                  results,
                  state,
               });
            },
            KeyCode::Up | KeyCode::BackTab => {
               move_list_up(&mut state);
               app.modal = Some(Modal::Search {
                  left,
                  right,
                  results,
                  state,
               });
            },
            KeyCode::Left => {
               if let Some(c) = left.pop() {
                  right.insert(0, c);
               }
               app.modal = Some(Modal::Search {
                  left,
                  right,
                  results,
                  state,
               });
            },
            KeyCode::Right => {
               if !right.is_empty() {
                  let c = right.remove(0);
                  left.push(c);
               }
               app.modal = Some(Modal::Search {
                  left,
                  right,
                  results,
                  state,
               });
            },
            KeyCode::Char(c) if is_search_char(c) => {
               left.push(c);
               let needle = format!("{left}{right}");
               results = run_search(&app.index, &needle);
               state = ListState::default();
               if !results.is_empty() {
                  state.select(Some(0));
               }
               app.modal = Some(Modal::Search {
                  left,
                  right,
                  results,
                  state,
               });
            },
            KeyCode::Backspace => {
               left.pop();
               let needle = format!("{left}{right}");
               results = run_search(&app.index, &needle);
               state = ListState::default();
               if !results.is_empty() {
                  state.select(Some(0));
               }
               app.modal = Some(Modal::Search {
                  left,
                  right,
                  results,
                  state,
               });
            },
            KeyCode::Enter => {
               if let Some(idx) = state.selected()
                  && let Some(&id) = results.get(idx)
               {
                  let route = shortest_path_to(&app.env, id);
                  app.select_route(route);
               }
            },
            _ => {
               app.modal = Some(Modal::Search {
                  left,
                  right,
                  results,
                  state,
               });
            },
         }
      },
   }
   false
}

fn move_list_down(state: &mut ListState, len: usize) {
   if len == 0 {
      return;
   }
   let new = state.selected().map_or(0, |i| (i + 1).min(len - 1));
   state.select(Some(new));
}

fn move_list_up(state: &mut ListState) {
   let new = state.selected().map_or(0, |i| i.saturating_sub(1));
   state.select(Some(new));
}

const fn is_search_char(c: char) -> bool {
   c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.' | '=' | '?' | '_')
}

fn path_display_len(env: &StoreEnv<PathStats>, path: &[NodeId]) -> usize {
   let mut total = 0;
   for (i, &id) in path.iter().enumerate() {
      total += env.get(id).name.short().chars().count();
      if i + 1 < path.len() {
         total += 3; // " → " — visually 3 columns
      }
   }
   total
}

fn draw(f: &mut Frame, app: &mut App) {
   let area = f.area();
   let chunks = Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Min(3), Constraint::Length(5)])
      .split(area);

   draw_panes(f, chunks[0], app);
   draw_info(f, chunks[1], app);

   if let Some(modal) = app.modal.as_mut() {
      let ctx = RenderCtx {
         env:   &app.env,
         units: app.units,
      };
      draw_modal(f, area, ctx, modal);
   }
}

fn draw_panes(f: &mut Frame, area: Rect, app: &mut App) {
   let block = Block::default().borders(Borders::ALL);
   let inner = block.inner(area);
   f.render_widget(block, area);

   app.last_page_size = inner.height as usize;

   let cols = Layout::default()
      .direction(Direction::Horizontal)
      .constraints([
         Constraint::Percentage(33),
         Constraint::Length(1),
         Constraint::Percentage(33),
         Constraint::Length(1),
         Constraint::Percentage(34),
      ])
      .split(inner);

   let highlight = app.highlight_active().then_some(app.sort_order);

   let ctx = RenderCtx {
      env:   &app.env,
      units: app.units,
   };
   render_pane(
      f,
      cols[0],
      ctx,
      &mut app.prev.state,
      &app.prev.items,
      None,
      false,
   );
   render_vbar(f, cols[1]);
   render_pane(
      f,
      cols[2],
      ctx,
      &mut app.curr.state,
      &app.curr.items,
      highlight,
      true,
   );
   render_vbar(f, cols[3]);
   render_pane(
      f,
      cols[4],
      ctx,
      &mut app.next.state,
      &app.next.items,
      None,
      false,
   );
}

fn render_vbar(f: &mut Frame, area: Rect) {
   let lines = std::iter::repeat_with(|| Line::from(Span::raw("│")))
      .take(area.height as usize)
      .collect::<Vec<Line>>();
   f.render_widget(Paragraph::new(lines), area);
}

/// The bits the per-row renderer needs that don't change between panes.
#[derive(Clone, Copy)]
struct RenderCtx<'a> {
   env:   &'a StoreEnv<PathStats>,
   units: SizeUnits,
}

fn render_pane(
   f: &mut Frame,
   area: Rect,
   ctx: RenderCtx<'_>,
   state: &mut ListState,
   items: &[NodeId],
   highlight_sort: Option<SortOrder>,
   focused: bool,
) {
   let rows = items
      .iter()
      .map(|&id| build_row(ctx, id, highlight_sort, area.width))
      .collect::<Vec<ListItem>>();

   let list = List::new(rows).highlight_style(if focused {
      Style::default().add_modifier(Modifier::REVERSED)
   } else {
      Style::default()
   });
   f.render_stateful_widget(list, area, state);
}

fn build_row(
   ctx: RenderCtx<'_>,
   id: NodeId,
   highlight_sort: Option<SortOrder>,
   width: u16,
) -> ListItem<'static> {
   let sp = ctx.env.get(id);
   let stats = &sp.payload;
   let is_leaf = sp.refs.is_empty();
   let row_color = if is_leaf {
      Style::default().fg(Color::Blue)
   } else {
      Style::default()
   };

   let sig_prefix = if sp.signatures.is_empty() {
      "  "
   } else {
      "✓ "
   };
   let label = sp
      .name
      .short_with_disambiguation(stats.disambiguation_chars);

   let total = pretty_size(stats.total_size, ctx.units);
   let added = pretty_size(stats.added_size, ctx.units);

   let underline = Style::default().add_modifier(Modifier::UNDERLINED);
   let under_if = |so: SortOrder| {
      if highlight_sort == Some(so) {
         underline
      } else {
         Style::default()
      }
   };

   let mut size_spans = Vec::<Span<'static>>::new();
   if is_leaf {
      let style = under_if(SortOrder::ClosureSize)
         .patch(under_if(SortOrder::AddedSize))
         .patch(row_color);
      size_spans.push(Span::styled(total, style));
   } else {
      size_spans.push(Span::styled(
         total,
         under_if(SortOrder::ClosureSize).patch(row_color),
      ));
      size_spans.push(Span::styled(" (", row_color));
      size_spans.push(Span::styled(
         added,
         under_if(SortOrder::AddedSize).patch(row_color),
      ));
      size_spans.push(Span::styled(")", row_color));
   }

   let size_visible_width = size_spans
      .iter()
      .map(|s| s.content.chars().count())
      .sum::<usize>();
   let label_width = label.chars().count();
   let prefix_width = sig_prefix.chars().count();

   let pad_target =
      (width as usize).saturating_sub(prefix_width + label_width + size_visible_width + 1);
   let padding = " ".repeat(pad_target.max(1));

   let mut spans = Vec::<Span<'static>>::new();
   spans.push(Span::styled(sig_prefix.to_string(), row_color));
   spans.push(Span::styled(
      label,
      under_if(SortOrder::Alphabetical).patch(row_color),
   ));
   spans.push(Span::raw(padding));
   spans.extend(size_spans);

   ListItem::new(Line::from(spans))
}

fn draw_info(f: &mut Frame, area: Rect, app: &App) {
   let Some(selected_id) = app.selected_id() else {
      return;
   };
   let selected = app.env.get(selected_id);
   let stats = &selected.payload;
   let highlight = app.highlight_active();

   let underline = Style::default().add_modifier(Modifier::UNDERLINED);
   let under_if = |so: SortOrder| {
      if highlight && app.sort_order == so {
         underline
      } else {
         Style::default()
      }
   };

   let (hash_part, short_part) = selected.name.split_short();
   let name_line = Line::from(vec![
      Span::raw(hash_part.to_string()),
      Span::styled(short_part.to_string(), under_if(SortOrder::Alphabetical)),
   ]);

   let nar_size = pretty_size(selected.size, app.units);
   let closure = pretty_size(stats.total_size, app.units);
   let added = pretty_size(stats.added_size, app.units);

   let sizes_line = Line::from(vec![
      Span::raw(format!("NAR Size: {nar_size}")),
      Span::raw(" | "),
      Span::styled(
         format!("Closure Size: {closure}"),
         under_if(SortOrder::ClosureSize),
      ),
      Span::raw(" | "),
      Span::styled(
         format!("Added Size: {added}"),
         under_if(SortOrder::AddedSize),
      ),
   ]);

   let sigs_text = if selected.signatures.is_empty() {
      "Signatures: ✗".to_string()
   } else {
      let names = selected
         .signatures
         .iter()
         .map(|s| s.key_name.clone())
         .collect::<Vec<String>>();
      format!("Signatures: {}", names.join(", "))
   };

   let parents_text = fmt_immediate_parents(&app.env, selected_id);

   let mut lines = vec![
      name_line,
      sizes_line,
      Line::from(sigs_text),
      Line::from(parents_text),
   ];
   if let Some(path) = app.yank_message() {
      lines.push(Line::from(Span::styled(
         format!("✓ Yanked: {path}"),
         Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
      )));
   }
   let p = Paragraph::new(lines).wrap(Wrap { trim: false });
   f.render_widget(p, area);
}

fn draw_modal(f: &mut Frame, screen: Rect, ctx: RenderCtx<'_>, modal: &mut Modal) {
   let area = centered_rect(90, 60, screen);
   f.render_widget(Clear, area);

   match modal {
      Modal::Notice { title, body } => {
         let block = Block::default().borders(Borders::ALL).title(title.as_str());
         let p = Paragraph::new(body.as_str())
            .block(block)
            .wrap(Wrap { trim: false });
         f.render_widget(p, area);
      },
      Modal::WhyDepends {
         paths,
         state,
         h_scroll,
      } => {
         let block = Block::default().borders(Borders::ALL).title("why-depends");
         let inner = block.inner(area);
         f.render_widget(block, area);
         let items = paths
            .iter()
            .map(|p| ListItem::new(render_path_line(ctx.env, p, *h_scroll)))
            .collect::<Vec<ListItem>>();
         let list =
            List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
         f.render_stateful_widget(list, inner, state);
      },
      Modal::Search {
         left,
         right,
         results,
         state,
      } => {
         let block = Block::default().borders(Borders::ALL).title("Search");
         let inner = block.inner(area);
         f.render_widget(block, area);
         let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(inner);
         let input_line = Line::from(vec![
            Span::raw(left.as_str()),
            Span::styled("|", Style::default().add_modifier(Modifier::REVERSED)),
            Span::raw(right.as_str()),
         ]);
         f.render_widget(Paragraph::new(input_line), chunks[0]);
         let row_width = chunks[1].width;
         let items = results
            .iter()
            .map(|&id| build_row(ctx, id, None, row_width))
            .collect::<Vec<ListItem>>();
         let list =
            List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
         f.render_stateful_widget(list, chunks[1], state);
      },
   }
}

fn render_path_line(env: &StoreEnv<PathStats>, path: &[NodeId], h_scroll: u16) -> Line<'static> {
   let mut full = String::new();
   for (i, &id) in path.iter().enumerate() {
      full.push_str(env.get(id).name.short());
      if i + 1 < path.len() {
         full.push_str(" → ");
      }
   }
   let skip = h_scroll as usize;
   let visible = full.chars().skip(skip).collect::<String>();
   Line::from(visible)
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
   let popup_layout = Layout::default()
      .direction(Direction::Vertical)
      .constraints([
         Constraint::Percentage((100 - percent_y) / 2),
         Constraint::Percentage(percent_y),
         Constraint::Percentage((100 - percent_y) / 2),
      ])
      .split(r);

   Layout::default()
      .direction(Direction::Horizontal)
      .constraints([
         Constraint::Percentage((100 - percent_x) / 2),
         Constraint::Percentage(percent_x),
         Constraint::Percentage((100 - percent_x) / 2),
      ])
      .split(popup_layout[1])[1]
}
