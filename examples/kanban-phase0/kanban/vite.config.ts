import { defineConfig, type Plugin } from 'vite';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import type { IncomingMessage, ServerResponse } from 'node:http';

/**
 * The todo.md file lives in the parent directory of this Vite project
 * (i.e. the directory where the kanban board was created).
 */
const HERE = path.dirname(fileURLToPath(import.meta.url));
const TODO_FILE = path.resolve(HERE, '..', 'todo.md');

export type TaskStatus = 'todo' | 'doing' | 'done';

export interface Task {
  id: string;
  text: string;
  status: TaskStatus;
}

const STATUS_TO_MARKER: Record<TaskStatus, string> = {
  todo: ' ',
  doing: '~',
  done: 'x',
};

const MARKER_TO_STATUS: Record<string, TaskStatus> = {
  ' ': 'todo',
  '~': 'doing',
  x: 'done',
  X: 'done',
};

/** Matches lines like "- [ ] text", "- [~] text", "- [x] text". */
const TASK_LINE = /^- \[([ xX~])\]\s*(.*)$/;

function parseTodo(content: string): Task[] {
  const tasks: Task[] = [];
  content.split(/\r?\n/).forEach((line, index) => {
    const match = TASK_LINE.exec(line.trim());
    if (!match) return;
    tasks.push({
      id: `t${index}`,
      text: match[2].trim(),
      status: MARKER_TO_STATUS[match[1]],
    });
  });
  return tasks;
}

function serializeTodo(tasks: Task[]): string {
  const lines = tasks.map((t) => `- [${STATUS_TO_MARKER[t.status]}] ${t.text}`);
  return lines.length > 0 ? `${lines.join('\n')}\n` : '';
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  if (res.headersSent) return;
  res.statusCode = status;
  res.setHeader('Content-Type', 'application/json; charset=utf-8');
  res.end(JSON.stringify(body));
}

function isTask(value: unknown): value is Task {
  if (typeof value !== 'object' || value === null) return false;
  const t = value as Task;
  return (
    typeof t.text === 'string' &&
    (t.status === 'todo' || t.status === 'doing' || t.status === 'done')
  );
}

/**
 * Tiny dev-server middleware that bridges the browser and ./todo.md:
 *   GET  /api/todo -> { tasks: Task[] }            (file order preserved)
 *   POST /api/todo <- { tasks: Task[] }            (rewrites the file)
 */
function todoApiPlugin(): Plugin {
  return {
    name: 'kanban-todo-api',
    configureServer(server) {
      server.middlewares.use(
        '/api/todo',
        (req: IncomingMessage, res: ServerResponse) => {
          if (req.method === 'GET') {
            try {
              const content = readFileSync(TODO_FILE, 'utf8');
              sendJson(res, 200, { tasks: parseTodo(content) });
            } catch (err) {
              sendJson(res, 500, {
                error: `Failed to read ${TODO_FILE}: ${String(err)}`,
              });
            }
            return;
          }

          if (req.method === 'POST') {
            let body = '';
            req.on('data', (chunk: Buffer) => {
              body += chunk;
            });
            req.on('end', () => {
              try {
                const parsed = JSON.parse(body) as { tasks?: unknown };
                const incoming = Array.isArray(parsed.tasks) ? parsed.tasks : [];
                const tasks = incoming.filter(isTask);
                writeFileSync(TODO_FILE, serializeTodo(tasks), 'utf8');
                sendJson(res, 200, {
                  ok: true,
                  tasks: parseTodo(readFileSync(TODO_FILE, 'utf8')),
                });
              } catch (err) {
                sendJson(res, 400, { error: `Invalid request: ${String(err)}` });
              }
            });
            req.on('error', () => {
              sendJson(res, 400, { error: 'Request stream error' });
            });
            return;
          }

          res.setHeader('Allow', 'GET, POST');
          sendJson(res, 405, { error: 'Method not allowed' });
        },
      );
    },
  };
}

export default defineConfig({
  plugins: [todoApiPlugin()],
});
