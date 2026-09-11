import './style.css';
import { initBoard } from './kanban';

initBoard(document.querySelector<HTMLDivElement>('#app')!);
