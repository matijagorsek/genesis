// pomodoro — 25:00 countdown with start/pause/reset and a beep at zero.
const DURATION = 25 * 60; // seconds

const clockEl = document.getElementById('clock');
const statusEl = document.getElementById('status');
const startBtn = document.getElementById('start');
const pauseBtn = document.getElementById('pause');
const resetBtn = document.getElementById('reset');

let remaining = DURATION;
let running = false;
let endTime = null;
let tickId = null;

function format(totalSeconds) {
  const m = Math.floor(totalSeconds / 60);
  const s = totalSeconds % 60;
  return String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0');
}

function render() {
  clockEl.textContent = format(remaining);
  document.title = running ? format(remaining) + ' — pomodoro' : 'pomodoro';
}

function beep() {
  try {
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = 'sine';
    osc.frequency.value = 880;
    gain.gain.setValueAtTime(0.3, ctx.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.4);
    osc.connect(gain).connect(ctx.destination);
    osc.start();
    osc.stop(ctx.currentTime + 0.4);
    osc.onended = () => ctx.close();
  } catch (e) {
    // Audio not available; fail silently.
  }
}

function tick() {
  remaining = Math.max(0, Math.round((endTime - Date.now()) / 1000));
  render();
  if (remaining <= 0) {
    stopTicking();
    running = false;
    statusEl.textContent = "Time's up!";
    startBtn.disabled = false;
    pauseBtn.disabled = true;
    beep();
  }
}

function stopTicking() {
  if (tickId !== null) {
    clearInterval(tickId);
    tickId = null;
  }
}

function start() {
  if (running) return;
  if (remaining <= 0) remaining = DURATION;
  running = true;
  endTime = Date.now() + remaining * 1000;
  statusEl.textContent = 'Focusing…';
  startBtn.disabled = true;
  pauseBtn.disabled = false;
  tickId = setInterval(tick, 250);
  tick();
}

function pause() {
  if (!running) return;
  stopTicking();
  running = false;
  statusEl.textContent = 'Paused.';
  startBtn.disabled = false;
  pauseBtn.disabled = true;
}

function reset() {
  stopTicking();
  running = false;
  remaining = DURATION;
  statusEl.textContent = 'Ready.';
  startBtn.disabled = false;
  pauseBtn.disabled = true;
  render();
}

startBtn.addEventListener('click', start);
pauseBtn.addEventListener('click', pause);
resetBtn.addEventListener('click', reset);

render();
