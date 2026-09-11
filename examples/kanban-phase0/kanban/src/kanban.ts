type TaskStatus = 'todo' | 'doing' | 'done';

interface Task {
  id: string;
  text: string;
  status: TaskStatus;
}

const COLUMNS: ReadonlyArray<{ status: TaskStatus; title: string }> = [
  { status: 'todo', title: 'Todo' },
  { status: 'doing', title: 'Doing' },
  { status: 'done', title: 'Done' },
];

type SaveState = 'idle' | 'saving' | 'saved' | 'error';

let root: HTMLElement | null = null;
let tasks: Task[] = [];
let saveTimer: number | undefined;

/* ---------------------------------- API ---------------------------------- */

async function fetchTasks(): Promise<Task[]> {
  const res = await fetch('/api/todo');
  if (!res.ok) throw new Error(`GET /api/todo -> ${res.status}`);
  const data = (await res.json()) as { tasks?: Task[] };
  return data.tasks ?? [];
}

async function saveTasks(next: Task[]): Promise<void> {
  setSaveState('saving');
  try {
    const res = await fetch('/api/todo', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ tasks: next }),
    });
    if (!res.ok) throw new Error(`POST /api/todo -> ${res.status}`);
    const data = (await res.json()) as { tasks?: Task[] };
    // Adopt the server's canonical copy (ids are line-index based).
    tasks = data.tasks ?? next;
    setSaveState('saved');
  } catch (err) {
    console.error('Failed to save todo.md', err);
    setSaveState('error');
  }
}

function queueSave(): void {
  if (saveTimer !== undefined) window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    saveTimer = undefined;
    void saveTasks(tasks);
  }, 150);
}

function setSaveState(state: SaveState): void {
  const el = document.querySelector<HTMLSpanElement>('#save-status');
  if (!el) return;
  el.dataset.state = state;
  el.textContent =
    state === 'saving'
      ? 'Saving…'
      : state === 'saved'
        ? 'Saved to todo.md'
        : state === 'error'
          ? 'Save failed — see console'
          : '';
}

/* -------------------------------- Rendering ------------------------------- */

function render(): void {
  if (!root) return;
  root.innerHTML = '';

  const board = document.createElement('main');
  board.className = 'board';

  for (const column of COLUMNS) {
    const columnTasks = tasks.filter((t) => t.status === column.status);

    const section = document.createElement('section');
    section.className = `column column-${column.status}`;

    const header = document.createElement('header');
    header.className = 'column-header';
    const title = document.createElement('h2');
    title.textContent = column.title;
    const count = document.createElement('span');
    count.className = 'count';
    count.textContent = String(columnTasks.length);
    header.append(title, count);

    const list = document.createElement('div');
    list.className = 'card-list';
    list.dataset.status = column.status;
    for (const task of columnTasks) {
      list.appendChild(createCard(task));
    }
    if (column.status === 'todo') {
      list.appendChild(createAddForm());
    }
    attachDropHandlers(list);

    section.append(header, list);
    board.appendChild(section);
  }

  root.appendChild(board);
}

function createCard(task: Task): HTMLElement {
  const card = document.createElement('div');
  card.className = 'card';
  card.draggable = true;
  card.dataset.id = task.id;
  card.textContent = task.text;

  card.addEventListener('dragstart', (e: DragEvent) => {
    if (e.dataTransfer) {
      e.dataTransfer.setData('text/plain', task.id);
      e.dataTransfer.effectAllowed = 'move';
    }
    // Defer the class change so the browser can snapshot the element first.
    requestAnimationFrame(() => card.classList.add('dragging'));
  });
  card.addEventListener('dragend', () => {
    card.classList.remove('dragging');
    clearDropHints();
  });
  return card;
}

function createAddForm(): HTMLFormElement {
  const form = document.createElement('form');
  form.className = 'add-task';
  const input = document.createElement('input');
  input.type = 'text';
  input.placeholder = 'Add a task…';
  input.setAttribute('aria-label', 'Add a task to Todo');
  form.appendChild(input);
  form.addEventListener('submit', (e: SubmitEvent) => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    tasks.push({ id: `new-${Date.now()}`, text, status: 'todo' });
    render();
    queueSave();
  });
  return form;
}

/* ------------------------------- Drag & drop ------------------------------ */

function attachDropHandlers(list: HTMLElement): void {
  list.addEventListener('dragover', (e: DragEvent) => {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    list.classList.add('drag-over');
  });
  list.addEventListener('dragleave', (e: DragEvent) => {
    if (!list.contains(e.relatedTarget as Node | null)) {
      list.classList.remove('drag-over');
    }
  });
  list.addEventListener('drop', (e: DragEvent) => {
    e.preventDefault();
    list.classList.remove('drag-over');
    const dragId = e.dataTransfer?.getData('text/plain') ?? '';
    if (!dragId) return;
    const after = getDragAfterElement(list, e.clientY);
    moveTask(dragId, list.dataset.status as TaskStatus, after?.dataset.id ?? null);
  });
}

/** The first non-dragged card whose vertical midpoint is below the cursor. */
function getDragAfterElement(list: HTMLElement, y: number): HTMLElement | null {
  const cards = Array.from(
    list.querySelectorAll<HTMLElement>('.card:not(.dragging)'),
  );
  let closest: { el: HTMLElement; offset: number } | null = null;
  for (const el of cards) {
    const box = el.getBoundingClientRect();
    const offset = y - box.top - box.height / 2;
    if (offset < 0 && (closest === null || offset > closest.offset)) {
      closest = { el, offset };
    }
  }
  return closest === null ? null : closest.el;
}

function clearDropHints(): void {
  document
    .querySelectorAll<HTMLElement>('.card-list.drag-over')
    .forEach((el) => el.classList.remove('drag-over'));
}

/**
 * Move a task to a column, optionally before another task.
 * The global `tasks` array is the canonical file order, so reordering within
 * a column and moving across columns both reduce to reinserting the task.
 */
function moveTask(dragId: string, toStatus: TaskStatus, beforeId: string | null): void {
  const task = tasks.find((t) => t.id === dragId);
  if (!task) return;

  const snapshot = (ts: Task[]) => ts.map((t) => `${t.id}:${t.status}`).join('|');
  const before = snapshot(tasks);

  task.status = toStatus;
  const rest = tasks.filter((t) => t.id !== dragId);
  if (beforeId === null) {
    // Append after the last task of the target column.
    let insertAt = rest.length;
    for (let i = rest.length - 1; i >= 0; i--) {
      if (rest[i].status === toStatus) {
        insertAt = i + 1;
        break;
      }
    }
    rest.splice(insertAt, 0, task);
  } else {
    const idx = rest.findIndex((t) => t.id === beforeId);
    rest.splice(idx === -1 ? rest.length : idx, 0, task);
  }

  tasks = rest;
  if (before === snapshot(tasks)) return; // dropped exactly where it was
  render();
  queueSave();
}

/* --------------------------------- Bootstrap ------------------------------ */

export function initBoard(appRoot: HTMLElement): void {
  root = appRoot;

  const topBar = document.createElement('header');
  topBar.className = 'topbar';
  const heading = document.createElement('h1');
  heading.textContent = 'Kanban';
  const status = document.createElement('span');
  status.id = 'save-status';
  status.dataset.state = 'idle';
  topBar.append(heading, status);
  appRoot.appendChild(topBar);

  void (async () => {
    try {
      tasks = await fetchTasks();
    } catch (err) {
      console.error('Failed to load todo.md', err);
      setSaveState('error');
      tasks = [];
    }
    render();
  })();
}
