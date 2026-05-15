const canvas = document.getElementById("radar-canvas");
const ctx = canvas.getContext("2d");
const radarMap = document.getElementById("radar-map");
const overlay = document.getElementById("overlay-message");
const playerList = document.getElementById("player-list");
const statusLine = document.getElementById("status-line");
const mapLine = document.getElementById("map-line");
const tickLine = document.getElementById("tick-line");
const windowLine = document.getElementById("window-line");
const serverLine = document.getElementById("server-line");
const shownLine = document.getElementById("shown-line");
const showTeammatesToggle = document.getElementById("show-teammates-toggle");
const showHealthToggle = document.getElementById("show-health-toggle");
const markerSizeValue = document.getElementById("marker-size-value");
const markerSizeDown = document.getElementById("marker-size-down");
const markerSizeUp = document.getElementById("marker-size-up");

const SETTINGS_KEY = "fun_radar_settings_v2";
const COLOR_STORAGE_KEY = "fun_radar_player_colors";
const POLL_INTERVAL_MS = 150;
const MARKER_SIZE_MIN = 8;
const MARKER_SIZE_MAX = 36;
const DEFAULT_COLORS = {
  self_player: "#ffffff",
  enemy: "#ff5a53",
  teammate: "#64a9ff",
  unknown: "#f0c24b",
};

let latestSnapshot = null;
let loadedImage = null;
let loadedImageName = null;
let pollTimer = null;
let customColors = loadColors();
let settings = loadSettings();

function loadSettings() {
  try {
    const raw = window.localStorage.getItem(SETTINGS_KEY);
    const parsed = raw ? JSON.parse(raw) : {};
    return {
      showTeammates: parsed.showTeammates ?? true,
      showHealthBars: parsed.showHealthBars ?? true,
      markerSize: clamp(parsed.markerSize ?? 16, MARKER_SIZE_MIN, MARKER_SIZE_MAX),
    };
  } catch {
    return {
      showTeammates: true,
      showHealthBars: true,
      markerSize: 16,
    };
  }
}

function saveSettings() {
  window.localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
}

function loadColors() {
  try {
    const raw = window.localStorage.getItem(COLOR_STORAGE_KEY);
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function saveColors() {
  window.localStorage.setItem(COLOR_STORAGE_KEY, JSON.stringify(customColors));
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function relationshipKey(player) {
  return String(player.relationship || "unknown").toLowerCase();
}

function colorKey(player) {
  return `${relationshipKey(player)}:${player.id}`;
}

function playerColor(player) {
  return customColors[colorKey(player)] || DEFAULT_COLORS[relationshipKey(player)] || "#ffffff";
}

function visiblePlayers(snapshot) {
  const players = snapshot?.players || [];
  if (settings.showTeammates) {
    return players;
  }
  return players.filter((player) => {
    const relationship = relationshipKey(player);
    return relationship === "self_player" || relationship !== "teammate";
  });
}

function setOverlay(message) {
  overlay.textContent = message;
  overlay.classList.add("visible");
}

function clearOverlay() {
  overlay.textContent = "";
  overlay.classList.remove("visible");
}

function syncSettingsUi() {
  showTeammatesToggle.checked = settings.showTeammates;
  showHealthToggle.checked = settings.showHealthBars;
  markerSizeValue.textContent = String(settings.markerSize);
}

function syncSummary(snapshot) {
  const counts = snapshot?.counts || {};
  statusLine.textContent = `status: ${String(snapshot?.status || "booting").toLowerCase()}`;
  mapLine.textContent = `map: ${snapshot?.map_key || "-"}`;
  tickLine.textContent = `tick: ${snapshot?.tick ?? "-"}`;
  windowLine.textContent = `window: ${snapshot?.gameplay_window || "-"}`;
  serverLine.textContent = `server players: ${counts.low_level_server_players ?? "-"}`;
  shownLine.textContent = `shown: ${visiblePlayers(snapshot).length}`;
}

function syncPlayerList(snapshot) {
  const players = visiblePlayers(snapshot);
  playerList.innerHTML = "";

  if (!players.length) {
    const empty = document.createElement("div");
    empty.className = "player-row empty";
    empty.textContent = "No players right now";
    playerList.appendChild(empty);
    return;
  }

  for (const player of players) {
    const row = document.createElement("label");
    row.className = "player-row";

    const dot = document.createElement("span");
    dot.className = "player-dot";
    dot.style.background = playerColor(player);

    const info = document.createElement("span");
    info.className = "player-name";
    const health = Number.isFinite(player.health) ? ` · hp ${player.health}` : "";
    info.innerHTML = `${player.name}<span class="player-meta">${relationshipKey(player)}${health}</span>`;

    const picker = document.createElement("input");
    picker.className = "player-color";
    picker.type = "color";
    picker.value = playerColor(player);
    picker.addEventListener("input", () => {
      customColors[colorKey(player)] = picker.value;
      saveColors();
      dot.style.background = picker.value;
    });

    row.append(dot, info, picker);
    playerList.appendChild(row);
  }
}

function loadMapImage(mapImage) {
  if (!mapImage) {
    loadedImage = null;
    loadedImageName = null;
    radarMap.classList.remove("visible");
    radarMap.removeAttribute("src");
    return;
  }

  if (loadedImageName === mapImage) {
    return;
  }

  const nextImage = new Image();
  nextImage.onload = () => {
    loadedImage = nextImage;
    loadedImageName = mapImage;
    radarMap.src = `/assets/${mapImage}`;
    radarMap.classList.add("visible");
  };
  nextImage.onerror = () => {
    loadedImage = null;
    loadedImageName = null;
    radarMap.classList.remove("visible");
    radarMap.removeAttribute("src");
    setOverlay(`missing map asset: ${mapImage}`);
  };
  nextImage.src = `/assets/${mapImage}`;
}

function handleSnapshot(snapshot) {
  latestSnapshot = snapshot;
  syncSummary(snapshot);
  syncPlayerList(snapshot);

  if (snapshot.status !== "ok" || !snapshot.map_image) {
    loadMapImage(null);
    setOverlay(snapshot.message || "waiting for data");
    return;
  }

  loadMapImage(snapshot.map_image);
  clearOverlay();
}

async function pollSnapshot() {
  try {
    const response = await fetch("/json", { cache: "no-store" });
    if (!response.ok) {
      throw new Error(`http ${response.status}`);
    }

    const snapshot = await response.json();
    handleSnapshot(snapshot);
  } catch (error) {
    if (!latestSnapshot) {
      setOverlay("waiting for radar data");
    }
    console.warn("fun-radar polling failed", error);
  } finally {
    pollTimer = window.setTimeout(pollSnapshot, POLL_INTERVAL_MS);
  }
}

function resizeCanvasToDisplaySize() {
  const ratio = window.devicePixelRatio || 1;
  const width = Math.floor(canvas.clientWidth * ratio);
  const height = Math.floor(canvas.clientHeight * ratio);
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
}

function drawMarker(player) {
  const x = player.x * canvas.clientWidth;
  const y = player.y * canvas.clientHeight;
  const radius = settings.markerSize / 2;
  const color = playerColor(player);
  const health = Number.isFinite(player.health) ? clamp(player.health, 0, 100) : null;
  const barWidth = settings.markerSize + 10;
  const barHeight = 5;
  const label = player.name || player.id;

  ctx.save();
  ctx.translate(x, y);

  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(0, 0, radius, 0, Math.PI * 2);
  ctx.fill();

  ctx.lineWidth = player.is_local ? 3 : 2;
  ctx.strokeStyle = player.is_local ? "rgba(255, 215, 64, 0.95)" : "rgba(10, 12, 16, 0.95)";
  ctx.stroke();

  if (player.is_local) {
    ctx.beginPath();
    ctx.arc(0, 0, radius + 5, 0, Math.PI * 2);
    ctx.strokeStyle = "rgba(255, 215, 64, 0.6)";
    ctx.lineWidth = 2;
    ctx.stroke();
  }

  ctx.font = '12px Consolas, "Courier New", monospace';
  ctx.textAlign = "center";
  ctx.textBaseline = "bottom";
  ctx.fillStyle = "#eef2f6";
  ctx.fillText(label, 0, -(radius + 8));

  if (settings.showHealthBars && health !== null) {
    const fill = health / 100;
    const barY = radius + 8;
    ctx.fillStyle = "rgba(10, 12, 16, 0.88)";
    ctx.fillRect(-(barWidth / 2), barY, barWidth, barHeight);
    ctx.fillStyle = health > 50 ? "#62d26f" : health > 20 ? "#f0c24b" : "#ff6b57";
    ctx.fillRect(-(barWidth / 2), barY, barWidth * fill, barHeight);
    ctx.lineWidth = 1;
    ctx.strokeStyle = "rgba(255,255,255,0.25)";
    ctx.strokeRect(-(barWidth / 2), barY, barWidth, barHeight);
  }

  ctx.restore();
}

function draw() {
  requestAnimationFrame(draw);
  resizeCanvasToDisplaySize();

  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  ctx.clearRect(0, 0, width, height);
  if (!loadedImage) {
    ctx.fillStyle = "#090b0f";
    ctx.fillRect(0, 0, width, height);
  }

  if (!latestSnapshot || latestSnapshot.status !== "ok") {
    return;
  }

  for (const player of visiblePlayers(latestSnapshot)) {
    drawMarker(player);
  }
}

function updateMarkerSize(delta) {
  settings.markerSize = clamp(settings.markerSize + delta, MARKER_SIZE_MIN, MARKER_SIZE_MAX);
  syncSettingsUi();
  saveSettings();
}

showTeammatesToggle.addEventListener("change", () => {
  settings.showTeammates = showTeammatesToggle.checked;
  syncSettingsUi();
  syncSummary(latestSnapshot);
  syncPlayerList(latestSnapshot);
  saveSettings();
});

showHealthToggle.addEventListener("change", () => {
  settings.showHealthBars = showHealthToggle.checked;
  syncSettingsUi();
  saveSettings();
});

markerSizeDown.addEventListener("click", () => updateMarkerSize(-2));
markerSizeUp.addEventListener("click", () => updateMarkerSize(2));

window.addEventListener("beforeunload", () => {
  window.clearTimeout(pollTimer);
});

syncSettingsUi();
syncSummary(null);
setOverlay("booting");
pollSnapshot();
draw();
