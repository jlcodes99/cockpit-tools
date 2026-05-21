import { Minus, Pause, Play, X } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ALL_PLATFORM_IDS, PlatformId } from '../../types/platform';
import { renderPlatformIcon } from '../../utils/platformMeta';
import { useEscClose } from '../../hooks/useEscClose';
import './BreakoutModal.css';

interface BreakoutModalProps {
  open: boolean;
  onMinimize: () => void;
  onTerminate: () => void;
}

type DropType = 'split' | 'triple' | 'expand' | 'shield';
const DROP_TYPES: DropType[] = ['split', 'triple', 'expand', 'shield'];
type DropCounts = Record<DropType, number>;
type DropIconMap = Record<DropType, PlatformId>;

interface Ball {
  id: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
  r: number;
}

interface Brick {
  id: number;
  x: number;
  y: number;
  w: number;
  h: number;
  alive: boolean;
}

interface DropItem {
  id: number;
  type: DropType;
  x: number;
  y: number;
  vy: number;
}

interface DropViewModel {
  id: number;
  type: DropType;
  x: number;
  y: number;
}

interface WallRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface BrickZone {
  x: number;
  y: number;
  cols: number;
  rows: number;
}

interface LevelLayout {
  walls: WallRect[];
  bricks: Brick[];
  brickLookup: Map<number, number>;
}

interface GameState {
  runSeed: number;
  level: number;
  levelScore: number;
  remainingBricks: number;
  paddleX: number;
  paddleWidth: number;
  isBallLaunched: boolean;
  expandUntil: number;
  walls: WallRect[];
  balls: Ball[];
  bricks: Brick[];
  brickLookup: Map<number, number>;
  drops: DropItem[];
  dropCounts: DropCounts;
  levelDropCounts: DropCounts;
  score: number;
  shields: number;
  nextBallId: number;
  nextDropId: number;
}

interface StageSize {
  width: number;
  height: number;
}

type GameEndReason = 'gameOver' | 'manualExit';

interface GameHistoryRecord {
  id: string;
  score: number;
  level: number;
  durationMs: number;
  createdAt: string;
  reason: GameEndReason;
  runSeed: number;
  dropCounts: DropCounts;
}

const BOARD_WIDTH = 760;
const BOARD_HEIGHT = 1080;
const PADDLE_BASE_WIDTH = 88;
const PADDLE_EXPAND_MAX_WIDTH = 168;
const PADDLE_EXPAND_DURATION_MS = 10000;
const PADDLE_HEIGHT = 11;
const PADDLE_Y = BOARD_HEIGHT - 120;
const PADDLE_SPEED = 10.5;
const MAX_BALLS = 300;
const MAX_DROPS = 60;
const BALL_RADIUS = 3;
const BALL_READY_Y = PADDLE_Y - BALL_RADIUS;
const OVERLAY_GUTTER = 36;
const MODAL_INNER_PADDING = 52;
const MODAL_EXTRA_HEIGHT = 32;
const BRICK_WIDTH = 6;
const BRICK_HEIGHT = 6;
const BRICK_GAP = 1;
const BRICK_RADIUS = Math.min(BRICK_WIDTH, BRICK_HEIGHT) / 2;
const BRICK_CELL_SIZE = BRICK_WIDTH + BRICK_GAP;
const BALL_STEP_DISTANCE = 3.5;
const BALL_COLLISION_CELL_SIZE = 18;
const BALL_COLLISION_RESTITUTION = 0.96;
const WALL_LAYER_COUNT = 3;
const WALL_LAYER_STEP_CELLS = 1;
const LEVEL_LAYOUT_STYLES = ['bandsHorizontal', 'bandsVertical', 'rings', 'triangles', 'diamonds', 'mixed'] as const;
const MIN_BRICK_COUNT = 2400;
const MIN_SEPARATOR_Y = 668;
const MAX_SEPARATOR_Y = 728;
const BREAKOUT_HISTORY_STORAGE_KEY = 'agtools.breakout.history';
const BREAKOUT_HISTORY_LIMIT = 200;
const BREAKOUT_DROP_ICON_ASSIGN_STORAGE_KEY = 'agtools.breakout.drop_icon_assign.v1';
const PLATFORM_LAYOUT_STORAGE_KEY = 'agtools.platform_layout.v1';
const FALLBACK_PLATFORM_ICON_ORDER: PlatformId[] = ['antigravity', 'codex', 'github-copilot', 'windsurf'];

const BASE_DROP_ICON_MAP: DropIconMap = {
  split: 'windsurf',
  triple: 'codex',
  expand: 'antigravity',
  shield: 'github-copilot',
};
const BASE_DROP_ICON_PLATFORM_IDS = Object.values(BASE_DROP_ICON_MAP) as PlatformId[];

const DROP_SPEED = 2.45;
const DROP_RATE_SPLIT = 0.15;
const DROP_RATE_TRIPLE = 0.15;
const DROP_RATE_EXPAND = 0.045;
const DROP_RATE_SHIELD = 0.045;
const DROP_CHANCE = DROP_RATE_SPLIT + DROP_RATE_TRIPLE + DROP_RATE_EXPAND + DROP_RATE_SHIELD;
const SPLIT_SHOT_SIDE_DELTA_VX = 2.6;
const SPLIT_SHOT_UP_SPEED = 6.3;
const TRIPLE_SHOT_SIDE_DELTA_VX = 2.2;
const TRIPLE_SHOT_MIN_UP_SPEED = 5.8;
const UI_SYNC_INTERVAL_MS = 34;
const BRICK_CELL_KEY_OFFSET = 512;
const BRICK_CELL_KEY_STRIDE = 2048;
const BALL_GRID_KEY_OFFSET = 256;
const BALL_GRID_KEY_STRIDE = 1024;
const INITIAL_SHIELDS = 3;

type LayoutStyle = (typeof LEVEL_LAYOUT_STYLES)[number];

function clamp(value: number, min: number, max: number): number {
  if (value < min) return min;
  if (value > max) return max;
  return value;
}

function randomInt(rng: () => number, min: number, max: number): number {
  return Math.floor(rng() * (max - min + 1)) + min;
}

function alignToBrickGrid(value: number): number {
  return Math.round(value / BRICK_CELL_SIZE) * BRICK_CELL_SIZE;
}

function buildCellKey(cellX: number, cellY: number): number {
  return (cellY + BRICK_CELL_KEY_OFFSET) * BRICK_CELL_KEY_STRIDE + (cellX + BRICK_CELL_KEY_OFFSET);
}

function buildBallGridKey(cellX: number, cellY: number): number {
  return (cellY + BALL_GRID_KEY_OFFSET) * BALL_GRID_KEY_STRIDE + (cellX + BALL_GRID_KEY_OFFSET);
}

function createEmptyDropCounts(): DropCounts {
  return {
    split: 0,
    triple: 0,
    expand: 0,
    shield: 0,
  };
}

function normalizeDropCounts(value: unknown): DropCounts {
  const result = createEmptyDropCounts();
  if (!value || typeof value !== 'object') return result;
  const candidate = value as Partial<Record<DropType, unknown>>;
  for (const type of DROP_TYPES) {
    const raw = candidate[type];
    if (typeof raw !== 'number' || !Number.isFinite(raw)) continue;
    result[type] = Math.max(0, Math.floor(raw));
  }
  return result;
}

function isPlatformId(value: unknown): value is PlatformId {
  return typeof value === 'string' && ALL_PLATFORM_IDS.includes(value as PlatformId);
}

function getDropTypeByPlatformId(platformId: PlatformId, dropIconMap: DropIconMap): DropType | null {
  for (const dropType of DROP_TYPES) {
    if (dropIconMap[dropType] === platformId) return dropType;
  }
  return null;
}

function normalizeDropIconMap(value: unknown): DropIconMap {
  const result: DropIconMap = { ...BASE_DROP_ICON_MAP };
  if (!value || typeof value !== 'object') return result;
  const candidate = value as Partial<Record<DropType, unknown>>;
  for (const dropType of DROP_TYPES) {
    const platformId = candidate[dropType];
    if (!isPlatformId(platformId)) continue;
    result[dropType] = platformId;
  }
  return result;
}

function normalizePlatformIdList(value: unknown): PlatformId[] {
  if (!Array.isArray(value)) return [];
  const deduped: PlatformId[] = [];
  for (const item of value) {
    if (!isPlatformId(item) || deduped.includes(item)) continue;
    deduped.push(item);
  }
  return deduped;
}

function mergePlatformIdList(target: PlatformId[], source: PlatformId[]): PlatformId[] {
  const next = [...target];
  for (const platformId of source) {
    if (next.includes(platformId)) continue;
    next.push(platformId);
  }
  return next;
}

function saveDropIconAssignmentState(
  dropIconMap: DropIconMap,
  seenPlatformIds: PlatformId[],
  nextDropTypeIndex: number,
) {
  try {
    localStorage.setItem(
      BREAKOUT_DROP_ICON_ASSIGN_STORAGE_KEY,
      JSON.stringify({
        dropIconMap,
        seenPlatformIds,
        nextDropTypeIndex,
      }),
    );
  } catch {
    // ignore localStorage write failure
  }
}

function loadBreakoutDropIconMap(): DropIconMap {
  let dropIconMap: DropIconMap = { ...BASE_DROP_ICON_MAP };
  let seenPlatformIds: PlatformId[] = [...BASE_DROP_ICON_PLATFORM_IDS];
  let nextDropTypeIndex = 0;

  try {
    const raw = localStorage.getItem(BREAKOUT_DROP_ICON_ASSIGN_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as {
        dropIconMap?: unknown;
        seenPlatformIds?: unknown;
        nextDropTypeIndex?: unknown;
      };

      dropIconMap = normalizeDropIconMap(parsed.dropIconMap);
      seenPlatformIds = normalizePlatformIdList(parsed.seenPlatformIds);

      if (typeof parsed.nextDropTypeIndex === 'number' && Number.isFinite(parsed.nextDropTypeIndex)) {
        const normalized = Math.floor(parsed.nextDropTypeIndex) % DROP_TYPES.length;
        nextDropTypeIndex = normalized < 0 ? normalized + DROP_TYPES.length : normalized;
      }
    }
  } catch {
    dropIconMap = { ...BASE_DROP_ICON_MAP };
    seenPlatformIds = [...BASE_DROP_ICON_PLATFORM_IDS];
    nextDropTypeIndex = 0;
  }

  seenPlatformIds = mergePlatformIdList(seenPlatformIds, BASE_DROP_ICON_PLATFORM_IDS);
  seenPlatformIds = mergePlatformIdList(seenPlatformIds, Object.values(dropIconMap) as PlatformId[]);

  for (const platformId of ALL_PLATFORM_IDS) {
    if (seenPlatformIds.includes(platformId)) continue;
    const dropType = DROP_TYPES[nextDropTypeIndex];
    dropIconMap[dropType] = platformId;
    seenPlatformIds.push(platformId);
    nextDropTypeIndex = (nextDropTypeIndex + 1) % DROP_TYPES.length;
  }

  saveDropIconAssignmentState(dropIconMap, seenPlatformIds, nextDropTypeIndex);
  return dropIconMap;
}

function buildDropTypeOrder(platformOrder: PlatformId[], dropIconMap: DropIconMap): DropType[] {
  const nextOrder: DropType[] = [];
  for (const platformId of platformOrder) {
    const dropType = getDropTypeByPlatformId(platformId, dropIconMap);
    if (!dropType || nextOrder.includes(dropType)) continue;
    nextOrder.push(dropType);
  }
  for (const dropType of DROP_TYPES) {
    if (!nextOrder.includes(dropType)) {
      nextOrder.push(dropType);
    }
  }
  return nextOrder;
}

function loadDropTypeOrderSnapshot(dropIconMap: DropIconMap): DropType[] {
  if (typeof window === 'undefined') {
    return buildDropTypeOrder(FALLBACK_PLATFORM_ICON_ORDER, dropIconMap);
  }
  try {
    const raw = window.localStorage.getItem(PLATFORM_LAYOUT_STORAGE_KEY);
    if (!raw) return buildDropTypeOrder(FALLBACK_PLATFORM_ICON_ORDER, dropIconMap);
    const parsed = JSON.parse(raw) as { orderedPlatformIds?: unknown };
    if (!Array.isArray(parsed.orderedPlatformIds)) {
      return buildDropTypeOrder(FALLBACK_PLATFORM_ICON_ORDER, dropIconMap);
    }
    const orderedPlatformIds = parsed.orderedPlatformIds.filter(isPlatformId);
    if (orderedPlatformIds.length === 0) {
      return buildDropTypeOrder(FALLBACK_PLATFORM_ICON_ORDER, dropIconMap);
    }
    return buildDropTypeOrder(orderedPlatformIds, dropIconMap);
  } catch {
    return buildDropTypeOrder(FALLBACK_PLATFORM_ICON_ORDER, dropIconMap);
  }
}

function getSortedDropTypes(dropCounts: DropCounts, defaultOrder: DropType[]): DropType[] {
  const indexMap = new Map<DropType, number>();
  for (let index = 0; index < defaultOrder.length; index += 1) {
    indexMap.set(defaultOrder[index], index);
  }

  return [...DROP_TYPES].sort((left, right) => {
    const diff = dropCounts[right] - dropCounts[left];
    if (diff !== 0) return diff;
    return (indexMap.get(left) ?? 999) - (indexMap.get(right) ?? 999);
  });
}

interface BrickArea {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

type SeparatorMode = 'none' | 'single' | 'double';

interface FrameSpec {
  left: number;
  right: number;
  top: number;
  separatorY: number;
  separatorMode: SeparatorMode;
}

function generateRunSeed(): number {
  return Math.floor(Math.random() * 0xffffffff) >>> 0;
}

function createLevelRng(runSeed: number, level: number): () => number {
  let seed = (runSeed ^ Math.imul(level + 1, 0x9e3779b1)) >>> 0;
  return () => {
    seed = (seed + 0x6d2b79f5) >>> 0;
    let t = seed;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function createFrameSpec(rng: () => number, level: number): FrameSpec {
  const left = alignToBrickGrid(randomInt(rng, 56, 92));
  const right = BOARD_WIDTH - left;
  const top = alignToBrickGrid(randomInt(rng, 78, 118));
  const separatorY = alignToBrickGrid(randomInt(rng, MIN_SEPARATOR_Y, MAX_SEPARATOR_Y));
  const roll = rng();
  let separatorMode: SeparatorMode = roll < 0.24 ? 'none' : roll < 0.72 ? 'single' : 'double';
  if (level <= 2 && separatorMode === 'double') {
    separatorMode = 'single';
  }
  return { left, right, top, separatorY, separatorMode };
}

function createOpening(
  frame: FrameSpec,
  center: number,
  width: number,
): { start: number; end: number } {
  const snappedWidth = Math.max(BRICK_CELL_SIZE * 8, Math.round(width / (BRICK_CELL_SIZE * 2)) * BRICK_CELL_SIZE * 2);
  const half = snappedWidth / 2;
  const minCenter = frame.left + half + 32;
  const maxCenter = frame.right - half - 32;
  const clampedCenter = clamp(alignToBrickGrid(center), minCenter, maxCenter);
  return {
    start: clampedCenter - half,
    end: clampedCenter + half,
  };
}

function createBottomSegments(frame: FrameSpec, rng: () => number): WallRect[] {
  if (frame.separatorMode === 'none') {
    return [
      {
        x: 0,
        y: frame.separatorY,
        w: frame.left + 6,
        h: 6,
      },
      {
        x: frame.right - 6,
        y: frame.separatorY,
        w: BOARD_WIDTH - frame.right + 6,
        h: 6,
      },
    ];
  }

  const width = frame.right - frame.left;
  const openings: Array<{ start: number; end: number }> = [];

  if (frame.separatorMode === 'single') {
    const center = frame.left + width / 2 + randomInt(rng, -9, 9) * BRICK_CELL_SIZE;
    openings.push(createOpening(frame, center, randomInt(rng, 96, 132)));
  } else {
    const centerA = frame.left + width * 0.33 + randomInt(rng, -6, 6) * BRICK_CELL_SIZE;
    const centerB = frame.left + width * 0.67 + randomInt(rng, -6, 6) * BRICK_CELL_SIZE;
    const openingA = createOpening(frame, centerA, randomInt(rng, 92, 122));
    const openingB = createOpening(frame, centerB, randomInt(rng, 92, 122));
    openings.push(openingA.start < openingB.start ? openingA : openingB);
    openings.push(openingA.start < openingB.start ? openingB : openingA);

    if (openings[1].start - openings[0].end < 56) {
      const push = (56 - (openings[1].start - openings[0].end)) / 2;
      openings[0].end -= push;
      openings[1].start += push;
    }
  }

  openings.sort((a, b) => a.start - b.start);
  const segments: WallRect[] = [];
  let cursor = 0;
  for (const opening of openings) {
    if (opening.start - cursor >= 6) {
      segments.push({
        x: cursor,
        y: frame.separatorY,
        w: opening.start - cursor,
        h: 6,
      });
    }
    cursor = opening.end;
  }
  if (BOARD_WIDTH - cursor >= 6) {
    segments.push({
      x: cursor,
      y: frame.separatorY,
      w: BOARD_WIDTH - cursor,
      h: 6,
    });
  }
  return segments;
}

function createPerimeterWalls(frame: FrameSpec, rng: () => number): WallRect[] {
  const baseWalls: WallRect[] = [
    { x: 0, y: frame.top - BRICK_CELL_SIZE, w: BOARD_WIDTH, h: 6 },
    { x: 0, y: frame.top, w: frame.left, h: frame.separatorY - frame.top },
    { x: frame.right, y: frame.top, w: BOARD_WIDTH - frame.right, h: frame.separatorY - frame.top },
    { x: frame.left, y: frame.top, w: 6, h: frame.separatorY - frame.top },
    { x: frame.right - 6, y: frame.top, w: 6, h: frame.separatorY - frame.top },
  ];
  baseWalls.push(...createBottomSegments(frame, rng));
  return baseWalls;
}

function createBrickArea(frame: FrameSpec, rng: () => number): BrickArea {
  const sidePadding = randomInt(rng, 4, 6) * BRICK_CELL_SIZE;
  const topPadding = randomInt(rng, 4, 7) * BRICK_CELL_SIZE;
  const bottomPadding = randomInt(rng, 3, 5) * BRICK_CELL_SIZE;
  const left = alignToBrickGrid(frame.left + sidePadding);
  const right = alignToBrickGrid(frame.right - sidePadding);
  const top = alignToBrickGrid(frame.top + topPadding);
  const bottom = alignToBrickGrid(Math.min(frame.separatorY - bottomPadding, 676));
  return { left, right, top, bottom };
}

function createStyleWalls(style: LayoutStyle, area: BrickArea, rng: () => number): WallRect[] {
  const width = area.right - area.left;
  const height = area.bottom - area.top;
  const midX = alignToBrickGrid((area.left + area.right) / 2);
  const minY = area.top + alignToBrickGrid(height * 0.18);
  const maxY = area.top + alignToBrickGrid(height * 0.58);
  const minH = alignToBrickGrid(height * 0.14);
  const maxH = alignToBrickGrid(height * 0.32);

  switch (style) {
    case 'bandsHorizontal':
      return [
        {
          x: midX - 3,
          y: clamp(area.top + alignToBrickGrid(height * 0.32 + randomInt(rng, -2, 2) * BRICK_CELL_SIZE), minY, maxY),
          w: 6,
          h: clamp(alignToBrickGrid(height * 0.16), minH, maxH),
        },
      ];
    case 'bandsVertical':
      return [
        {
          x: area.left + alignToBrickGrid(width * 0.34) - 3,
          y: clamp(area.top + alignToBrickGrid(height * 0.22), minY, maxY),
          w: 6,
          h: clamp(alignToBrickGrid(height * 0.28), minH, maxH),
        },
        {
          x: area.left + alignToBrickGrid(width * 0.66) - 3,
          y: clamp(area.top + alignToBrickGrid(height * 0.36), minY, maxY),
          w: 6,
          h: clamp(alignToBrickGrid(height * 0.26), minH, maxH),
        },
      ];
    case 'rings':
      return [
        {
          x: midX - 3 + randomInt(rng, -2, 2) * BRICK_CELL_SIZE,
          y: clamp(area.top + alignToBrickGrid(height * 0.2), minY, maxY),
          w: 6,
          h: clamp(alignToBrickGrid(height * 0.22), minH, maxH),
        },
      ];
    case 'triangles':
      return [
        {
          x: midX - 3,
          y: clamp(area.top + alignToBrickGrid(height * 0.2), minY, maxY),
          w: 6,
          h: clamp(alignToBrickGrid(height * 0.24), minH, maxH),
        },
        {
          x: area.left + alignToBrickGrid(width * 0.52) - 3,
          y: clamp(area.top + alignToBrickGrid(height * 0.42), minY, maxY),
          w: 6,
          h: clamp(alignToBrickGrid(height * 0.18), minH, maxH),
        },
      ];
    case 'diamonds':
      return [
        {
          x: area.left + alignToBrickGrid(width * 0.4) - 3,
          y: clamp(area.top + alignToBrickGrid(height * 0.3), minY, maxY),
          w: 6,
          h: clamp(alignToBrickGrid(height * 0.2), minH, maxH),
        },
        {
          x: area.left + alignToBrickGrid(width * 0.6) - 3,
          y: clamp(area.top + alignToBrickGrid(height * 0.24), minY, maxY),
          w: 6,
          h: clamp(alignToBrickGrid(height * 0.2), minH, maxH),
        }
      ];
    case 'mixed':
      if (rng() < 0.34) {
        return [];
      }
      if (rng() < 0.67) {
        return [{
          x: area.left + alignToBrickGrid(width * (0.3 + rng() * 0.4)) - 3,
          y: randomInt(rng, minY, maxY),
          w: 6,
          h: randomInt(rng, minH, maxH),
        }];
      }
      return [
        {
          x: area.left + alignToBrickGrid(width * 0.36) - 3,
          y: randomInt(rng, minY, maxY),
          w: 6,
          h: randomInt(rng, minH, maxH),
        },
        {
          x: area.left + alignToBrickGrid(width * 0.64) - 3,
          y: randomInt(rng, minY, maxY),
          w: 6,
          h: randomInt(rng, minH, maxH),
        },
      ];
    default:
      return [];
  }
}

function createBoardGrid(): PatternGrid {
  const cols = Math.max(1, Math.ceil(BOARD_WIDTH / BRICK_CELL_SIZE));
  const rows = Math.max(1, Math.ceil(BOARD_HEIGHT / BRICK_CELL_SIZE));
  return {
    cols,
    rows,
    cells: new Uint8Array(cols * rows),
  };
}

function stampWallRectToGrid(grid: PatternGrid, wall: WallRect) {
  const minCol = clamp(Math.floor((wall.x - BRICK_WIDTH) / BRICK_CELL_SIZE), 0, grid.cols - 1);
  const maxCol = clamp(Math.floor((wall.x + wall.w - 1) / BRICK_CELL_SIZE), 0, grid.cols - 1);
  const minRow = clamp(Math.floor((wall.y - BRICK_HEIGHT) / BRICK_CELL_SIZE), 0, grid.rows - 1);
  const maxRow = clamp(Math.floor((wall.y + wall.h - 1) / BRICK_CELL_SIZE), 0, grid.rows - 1);

  for (let row = minRow; row <= maxRow; row += 1) {
    for (let col = minCol; col <= maxCol; col += 1) {
      const cellRect = {
        x: col * BRICK_CELL_SIZE,
        y: row * BRICK_CELL_SIZE,
        w: BRICK_WIDTH,
        h: BRICK_HEIGHT,
      };
      if (isRectOverlap(cellRect, wall)) {
        setGridCell(grid, col, row, 1);
      }
    }
  }
}

function wallGridToRects(grid: PatternGrid): WallRect[] {
  const zones = convertGridToZones(grid, {
    left: 0,
    right: BOARD_WIDTH,
    top: 0,
    bottom: BOARD_HEIGHT,
  });
  const walls: WallRect[] = [];

  for (const zone of zones) {
    if (zone.x >= BOARD_WIDTH || zone.y >= BOARD_HEIGHT) continue;
    const width = Math.min(zone.cols * BRICK_CELL_SIZE, BOARD_WIDTH - zone.x);
    const height = Math.min(zone.rows * BRICK_CELL_SIZE, BOARD_HEIGHT - zone.y);
    if (width <= 0 || height <= 0) continue;
    walls.push({
      x: zone.x,
      y: zone.y,
      w: width,
      h: height,
    });
  }

  return walls;
}

function layerWalls(baseWalls: WallRect[]): WallRect[] {
  const wallGrid = createBoardGrid();

  for (const wall of baseWalls) {
    const isSolidBlock = wall.w > BRICK_WIDTH && wall.h > BRICK_HEIGHT;
    const layerCount = isSolidBlock ? 1 : WALL_LAYER_COUNT;
    const horizontal = wall.w >= wall.h;
    let shiftXCells = 0;
    let shiftYCells = 0;

    if (!isSolidBlock) {
      if (horizontal) {
        shiftYCells = wall.y <= BOARD_HEIGHT / 2 ? -WALL_LAYER_STEP_CELLS : WALL_LAYER_STEP_CELLS;
      } else {
        const isLeftPerimeter = wall.x < BOARD_WIDTH * 0.25;
        const isRightPerimeter = wall.x > BOARD_WIDTH * 0.75;
        if (isLeftPerimeter) {
          shiftXCells = WALL_LAYER_STEP_CELLS;
        } else if (isRightPerimeter) {
          shiftXCells = -WALL_LAYER_STEP_CELLS;
        } else {
          shiftXCells = wall.x <= BOARD_WIDTH / 2 ? -WALL_LAYER_STEP_CELLS : WALL_LAYER_STEP_CELLS;
        }
      }
    }

    for (let layer = 0; layer < layerCount; layer += 1) {
      stampWallRectToGrid(wallGrid, {
        x: wall.x + shiftXCells * layer * BRICK_CELL_SIZE,
        y: wall.y + shiftYCells * layer * BRICK_CELL_SIZE,
        w: wall.w,
        h: wall.h,
      });
    }
  }

  return wallGridToRects(wallGrid);
}

function pushZone(zones: BrickZone[], x: number, y: number, cols: number, rows: number) {
  if (cols <= 0 || rows <= 0) return;
  zones.push({
    x: alignToBrickGrid(x),
    y: alignToBrickGrid(y),
    cols,
    rows,
  });
}

function createFallbackZones(area: BrickArea): BrickZone[] {
  const zones: BrickZone[] = [];
  const areaCols = Math.max(32, Math.floor((area.right - area.left) / BRICK_CELL_SIZE));
  const areaRows = Math.max(44, Math.floor((area.bottom - area.top) / BRICK_CELL_SIZE));
  const slabRows = 8;
  const slabCount = 6;
  const slabGapRows = Math.max(3, Math.floor((areaRows - slabCount * slabRows) / Math.max(1, slabCount - 1)));
  const centerGapCols = 10;
  const slabCols = Math.max(30, Math.floor((areaCols - centerGapCols) / 2));
  const leftStartCol = Math.max(0, Math.floor((areaCols - (slabCols * 2 + centerGapCols)) / 2));
  const rightStartCol = leftStartCol + slabCols + centerGapCols;

  for (let slab = 0; slab < slabCount; slab += 1) {
    const rowStart = slab * (slabRows + slabGapRows);
    pushZone(
      zones,
      area.left + leftStartCol * BRICK_CELL_SIZE,
      area.top + rowStart * BRICK_CELL_SIZE,
      slabCols,
      slabRows,
    );
    pushZone(
      zones,
      area.left + rightStartCol * BRICK_CELL_SIZE,
      area.top + rowStart * BRICK_CELL_SIZE,
      slabCols,
      slabRows,
    );
  }
  return zones;
}

interface PatternGrid {
  cols: number;
  rows: number;
  cells: Uint8Array;
}

function createPatternGrid(area: BrickArea): PatternGrid {
  const cols = Math.max(24, Math.floor((area.right - area.left) / BRICK_CELL_SIZE));
  const rows = Math.max(28, Math.floor((area.bottom - area.top) / BRICK_CELL_SIZE));
  return {
    cols,
    rows,
    cells: new Uint8Array(cols * rows),
  };
}

function gridIndex(grid: PatternGrid, col: number, row: number): number {
  return row * grid.cols + col;
}

function setGridCell(grid: PatternGrid, col: number, row: number, value: 0 | 1) {
  if (col < 0 || col >= grid.cols || row < 0 || row >= grid.rows) return;
  grid.cells[gridIndex(grid, col, row)] = value;
}

function fillRectCells(grid: PatternGrid, x: number, y: number, w: number, h: number) {
  const startX = clamp(Math.floor(x), 0, grid.cols - 1);
  const endX = clamp(Math.floor(x + w - 1), 0, grid.cols - 1);
  const startY = clamp(Math.floor(y), 0, grid.rows - 1);
  const endY = clamp(Math.floor(y + h - 1), 0, grid.rows - 1);
  if (endX < startX || endY < startY) return;

  for (let row = startY; row <= endY; row += 1) {
    for (let col = startX; col <= endX; col += 1) {
      setGridCell(grid, col, row, 1);
    }
  }
}

function fillEllipseCells(
  grid: PatternGrid,
  centerX: number,
  centerY: number,
  radiusX: number,
  radiusY: number,
) {
  if (radiusX <= 0 || radiusY <= 0) return;
  const minX = Math.floor(centerX - radiusX);
  const maxX = Math.ceil(centerX + radiusX);
  const minY = Math.floor(centerY - radiusY);
  const maxY = Math.ceil(centerY + radiusY);

  for (let row = minY; row <= maxY; row += 1) {
    for (let col = minX; col <= maxX; col += 1) {
      const dx = (col + 0.5 - centerX) / radiusX;
      const dy = (row + 0.5 - centerY) / radiusY;
      if (dx * dx + dy * dy <= 1) {
        setGridCell(grid, col, row, 1);
      }
    }
  }
}

function fillRingCells(
  grid: PatternGrid,
  centerX: number,
  centerY: number,
  radiusX: number,
  radiusY: number,
  thickness: number,
) {
  if (radiusX <= 0 || radiusY <= 0) return;
  const innerX = Math.max(0.5, radiusX - thickness);
  const innerY = Math.max(0.5, radiusY - thickness);
  const minX = Math.floor(centerX - radiusX);
  const maxX = Math.ceil(centerX + radiusX);
  const minY = Math.floor(centerY - radiusY);
  const maxY = Math.ceil(centerY + radiusY);

  for (let row = minY; row <= maxY; row += 1) {
    for (let col = minX; col <= maxX; col += 1) {
      const outerDx = (col + 0.5 - centerX) / radiusX;
      const outerDy = (row + 0.5 - centerY) / radiusY;
      const innerDx = (col + 0.5 - centerX) / innerX;
      const innerDy = (row + 0.5 - centerY) / innerY;
      const inOuter = outerDx * outerDx + outerDy * outerDy <= 1;
      const inInner = innerDx * innerDx + innerDy * innerDy < 1;
      if (inOuter && !inInner) {
        setGridCell(grid, col, row, 1);
      }
    }
  }
}

function fillDiamondCells(grid: PatternGrid, centerX: number, centerY: number, radius: number) {
  if (radius <= 0) return;
  const minX = Math.floor(centerX - radius);
  const maxX = Math.ceil(centerX + radius);
  const minY = Math.floor(centerY - radius);
  const maxY = Math.ceil(centerY + radius);

  for (let row = minY; row <= maxY; row += 1) {
    for (let col = minX; col <= maxX; col += 1) {
      const dx = Math.abs(col + 0.5 - centerX);
      const dy = Math.abs(row + 0.5 - centerY);
      if ((dx + dy) / radius <= 1) {
        setGridCell(grid, col, row, 1);
      }
    }
  }
}

function fillTriangleCells(
  grid: PatternGrid,
  centerX: number,
  apexY: number,
  width: number,
  height: number,
  direction: 'up' | 'down',
) {
  if (width <= 0 || height <= 0) return;
  const halfWidth = width / 2;

  for (let i = 0; i < height; i += 1) {
    const progress = height <= 1 ? 1 : i / (height - 1);
    const span = Math.max(1, Math.round(halfWidth * progress));
    const y = direction === 'up' ? Math.round(apexY + i) : Math.round(apexY - i);
    fillRectCells(grid, centerX - span, y, span * 2, 1);
  }
}

function countFilledCells(grid: PatternGrid): number {
  let count = 0;
  for (let index = 0; index < grid.cells.length; index += 1) {
    if (grid.cells[index] === 1) count += 1;
  }
  return count;
}

function removeSmallIslands(grid: PatternGrid, minSize: number) {
  const visited = new Uint8Array(grid.cells.length);
  const stack: number[] = [];
  const component: number[] = [];
  const dirs = [-1, 1, -grid.cols, grid.cols];

  for (let index = 0; index < grid.cells.length; index += 1) {
    if (grid.cells[index] === 0 || visited[index] === 1) continue;
    stack.length = 0;
    component.length = 0;
    stack.push(index);
    visited[index] = 1;

    while (stack.length > 0) {
      const node = stack.pop();
      if (node == null) continue;
      component.push(node);
      const row = Math.floor(node / grid.cols);
      const col = node % grid.cols;

      for (const dir of dirs) {
        const next = node + dir;
        if (next < 0 || next >= grid.cells.length) continue;
        const nextRow = Math.floor(next / grid.cols);
        const nextCol = next % grid.cols;
        if (Math.abs(nextRow - row) + Math.abs(nextCol - col) !== 1) continue;
        if (grid.cells[next] === 0 || visited[next] === 1) continue;
        visited[next] = 1;
        stack.push(next);
      }
    }

    if (component.length < minSize) {
      for (const node of component) {
        grid.cells[node] = 0;
      }
    }
  }
}

function ensureDensePattern(grid: PatternGrid, rng: () => number, level: number) {
  const totalCells = grid.cols * grid.rows;
  const targetDensity = clamp(0.5 + level * 0.012, 0.52, 0.68);
  const target = Math.floor(totalCells * targetDensity);
  let filled = countFilledCells(grid);

  let guard = 0;
  while (filled < target && guard < 120) {
    guard += 1;
    const chunkW = randomInt(rng, Math.max(10, Math.floor(grid.cols * 0.16)), Math.max(18, Math.floor(grid.cols * 0.34)));
    const chunkH = randomInt(rng, Math.max(6, Math.floor(grid.rows * 0.08)), Math.max(14, Math.floor(grid.rows * 0.2)));
    const x = randomInt(rng, 1, Math.max(1, grid.cols - chunkW - 1));
    const y = randomInt(rng, 1, Math.max(1, grid.rows - chunkH - 1));
    fillRectCells(grid, x, y, chunkW, chunkH);
    filled = countFilledCells(grid);
  }

  const maxDensity = 0.74;
  if (filled > totalCells * maxDensity) {
    const carveCount = randomInt(rng, 2, 4);
    for (let i = 0; i < carveCount; i += 1) {
      const holeW = randomInt(rng, Math.max(6, Math.floor(grid.cols * 0.08)), Math.max(12, Math.floor(grid.cols * 0.16)));
      const holeH = randomInt(rng, Math.max(4, Math.floor(grid.rows * 0.06)), Math.max(10, Math.floor(grid.rows * 0.14)));
      const x = randomInt(rng, 2, Math.max(2, grid.cols - holeW - 2));
      const y = randomInt(rng, 2, Math.max(2, grid.rows - holeH - 2));
      for (let row = y; row < y + holeH; row += 1) {
        for (let col = x; col < x + holeW; col += 1) {
          setGridCell(grid, col, row, 0);
        }
      }
    }
  }
}

function drawHorizontalBands(grid: PatternGrid, rng: () => number) {
  const bandCount = randomInt(rng, 6, 9);
  const bandRows = randomInt(rng, 6, 9);
  const gapRows = Math.max(2, Math.floor((grid.rows - bandCount * bandRows) / Math.max(1, bandCount)));
  const usedRows = bandCount * bandRows + (bandCount - 1) * gapRows;
  let rowCursor = Math.max(1, Math.floor((grid.rows - usedRows) / 2));

  for (let band = 0; band < bandCount; band += 1) {
    const sideInset = randomInt(rng, 1, 4);
    const centerGap = randomInt(rng, 8, 14);
    const split = randomInt(rng, -4, 4);
    const baseHalf = Math.floor((grid.cols - centerGap) / 2);
    const leftWidth = clamp(baseHalf + split, 12, grid.cols - centerGap - 12);
    const rightWidth = grid.cols - centerGap - leftWidth;
    fillRectCells(grid, sideInset, rowCursor, leftWidth - sideInset, bandRows);
    fillRectCells(grid, grid.cols - rightWidth, rowCursor, rightWidth - sideInset, bandRows);

    if (rng() < 0.34) {
      const bridgeWidth = randomInt(rng, 5, 10);
      fillRectCells(
        grid,
        Math.floor((grid.cols - bridgeWidth) / 2),
        rowCursor + Math.floor(bandRows * 0.3),
        bridgeWidth,
        Math.max(2, Math.floor(bandRows * 0.45)),
      );
    }
    rowCursor += bandRows + gapRows;
  }
}

function drawVerticalBands(grid: PatternGrid, rng: () => number) {
  const pillarCount = randomInt(rng, 5, 8);
  const pillarCols = randomInt(rng, 6, 9);
  const gapCols = Math.max(2, Math.floor((grid.cols - pillarCount * pillarCols) / Math.max(1, pillarCount)));
  const usedCols = pillarCount * pillarCols + (pillarCount - 1) * gapCols;
  let colCursor = Math.max(1, Math.floor((grid.cols - usedCols) / 2));

  for (let pillar = 0; pillar < pillarCount; pillar += 1) {
    const topInset = randomInt(rng, 1, 5);
    const bottomInset = randomInt(rng, 5, 11);
    fillRectCells(grid, colCursor, topInset, pillarCols, grid.rows - topInset - bottomInset);
    colCursor += pillarCols + gapCols;
  }

  const bridges = randomInt(rng, 2, 4);
  for (let bridge = 0; bridge < bridges; bridge += 1) {
    const y = randomInt(rng, Math.floor(grid.rows * 0.2), Math.floor(grid.rows * 0.78));
    const width = randomInt(rng, Math.floor(grid.cols * 0.45), Math.floor(grid.cols * 0.8));
    fillRectCells(grid, Math.floor((grid.cols - width) / 2), y, width, randomInt(rng, 2, 4));
  }
}

function drawRings(grid: PatternGrid, rng: () => number) {
  const cx = grid.cols / 2 + randomInt(rng, -4, 4);
  const cy = grid.rows * (0.42 + rng() * 0.14);
  const rx = Math.floor(grid.cols * (0.28 + rng() * 0.11));
  const ry = Math.floor(grid.rows * (0.25 + rng() * 0.11));
  const thickness = randomInt(rng, 4, 7);
  fillRingCells(grid, cx, cy, rx, ry, thickness);
  fillEllipseCells(grid, cx, cy, Math.floor(rx * 0.35), Math.floor(ry * 0.35));

  if (rng() < 0.72) {
    fillRingCells(
      grid,
      cx + randomInt(rng, -8, 8),
      cy + randomInt(rng, -6, 6),
      Math.floor(rx * 0.65),
      Math.floor(ry * 0.62),
      Math.max(3, thickness - 2),
    );
  }

  fillRectCells(grid, Math.floor(grid.cols * 0.18), Math.floor(grid.rows * 0.7), Math.floor(grid.cols * 0.64), randomInt(rng, 4, 7));
}

function drawTriangles(grid: PatternGrid, rng: () => number) {
  const mainWidth = Math.floor(grid.cols * (0.66 + rng() * 0.16));
  const mainHeight = Math.floor(grid.rows * (0.55 + rng() * 0.2));
  fillTriangleCells(
    grid,
    grid.cols * 0.5 + randomInt(rng, -4, 4),
    Math.floor(grid.rows * 0.16),
    mainWidth,
    mainHeight,
    'up',
  );

  const sideWidth = Math.floor(grid.cols * (0.26 + rng() * 0.08));
  const sideHeight = Math.floor(grid.rows * (0.22 + rng() * 0.08));
  fillTriangleCells(grid, grid.cols * 0.24, Math.floor(grid.rows * 0.74), sideWidth, sideHeight, 'down');
  fillTriangleCells(grid, grid.cols * 0.76, Math.floor(grid.rows * 0.74), sideWidth, sideHeight, 'down');
  fillRectCells(grid, Math.floor(grid.cols * 0.16), Math.floor(grid.rows * 0.78), Math.floor(grid.cols * 0.68), randomInt(rng, 3, 5));
}

function drawDiamonds(grid: PatternGrid, rng: () => number) {
  const mainRadius = Math.floor(Math.min(grid.cols, grid.rows) * (0.3 + rng() * 0.08));
  fillDiamondCells(grid, grid.cols * 0.5, grid.rows * 0.42, mainRadius);
  fillDiamondCells(grid, grid.cols * 0.26, grid.rows * 0.64, Math.floor(mainRadius * 0.56));
  fillDiamondCells(grid, grid.cols * 0.74, grid.rows * 0.64, Math.floor(mainRadius * 0.56));

  if (rng() < 0.65) {
    fillDiamondCells(
      grid,
      grid.cols * 0.5 + randomInt(rng, -4, 4),
      grid.rows * 0.78,
      Math.floor(mainRadius * 0.45),
    );
  }
}

function drawMixed(grid: PatternGrid, rng: () => number) {
  const clusterCount = randomInt(rng, 2, 4);
  for (let cluster = 0; cluster < clusterCount; cluster += 1) {
    const anchorX = randomInt(rng, Math.floor(grid.cols * 0.2), Math.floor(grid.cols * 0.8));
    const anchorY = randomInt(rng, Math.floor(grid.rows * 0.18), Math.floor(grid.rows * 0.78));
    const stamps = randomInt(rng, 3, 5);
    for (let stamp = 0; stamp < stamps; stamp += 1) {
      const typeRoll = rng();
      const x = anchorX + randomInt(rng, -8, 8);
      const y = anchorY + randomInt(rng, -7, 7);
      if (typeRoll < 0.3) {
        fillRectCells(
          grid,
          x - randomInt(rng, 8, 16),
          y - randomInt(rng, 4, 8),
          randomInt(rng, 14, 24),
          randomInt(rng, 6, 12),
        );
      } else if (typeRoll < 0.55) {
        fillEllipseCells(grid, x, y, randomInt(rng, 8, 14), randomInt(rng, 6, 11));
      } else if (typeRoll < 0.8) {
        fillDiamondCells(grid, x, y, randomInt(rng, 8, 14));
      } else {
        fillTriangleCells(grid, x, y + randomInt(rng, -4, 4), randomInt(rng, 14, 22), randomInt(rng, 10, 16), rng() < 0.5 ? 'up' : 'down');
      }
    }
  }
}

function populatePattern(style: LayoutStyle, grid: PatternGrid, level: number, rng: () => number) {
  switch (style) {
    case 'bandsHorizontal':
      drawHorizontalBands(grid, rng);
      break;
    case 'bandsVertical':
      drawVerticalBands(grid, rng);
      break;
    case 'rings':
      drawRings(grid, rng);
      break;
    case 'triangles':
      drawTriangles(grid, rng);
      break;
    case 'diamonds':
      drawDiamonds(grid, rng);
      break;
    case 'mixed':
      drawMixed(grid, rng);
      break;
    default:
      drawHorizontalBands(grid, rng);
      break;
  }

  removeSmallIslands(grid, 24);
  ensureDensePattern(grid, rng, level);
  removeSmallIslands(grid, 20);
}

function convertGridToZones(grid: PatternGrid, area: BrickArea): BrickZone[] {
  const zones: BrickZone[] = [];
  const active = new Map<string, BrickZone>();

  for (let row = 0; row < grid.rows; row += 1) {
    const next = new Map<string, BrickZone>();
    let col = 0;
    while (col < grid.cols) {
      const index = gridIndex(grid, col, row);
      if (grid.cells[index] === 0) {
        col += 1;
        continue;
      }

      const start = col;
      while (col < grid.cols && grid.cells[gridIndex(grid, col, row)] === 1) {
        col += 1;
      }
      const span = col - start;
      const key = `${start}:${span}`;
      const existing = active.get(key);
      if (existing) {
        existing.rows += 1;
        next.set(key, existing);
      } else {
        const zone: BrickZone = {
          x: area.left + start * BRICK_CELL_SIZE,
          y: area.top + row * BRICK_CELL_SIZE,
          cols: span,
          rows: 1,
        };
        zones.push(zone);
        next.set(key, zone);
      }
    }
    active.clear();
    for (const [key, zone] of next.entries()) {
      active.set(key, zone);
    }
  }

  return zones;
}

function createBrickZones(
  style: LayoutStyle,
  area: BrickArea,
  level: number,
  rng: () => number,
): BrickZone[] {
  const grid = createPatternGrid(area);
  populatePattern(style, grid, level, rng);
  return convertGridToZones(grid, area);
}

function isRectOverlap(a: { x: number; y: number; w: number; h: number }, b: { x: number; y: number; w: number; h: number }): boolean {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
}

function buildBricksFromZones(
  zones: BrickZone[],
  walls: WallRect[],
  area: BrickArea,
): { bricks: Brick[]; brickLookup: Map<number, number> } {
  const usedCells = new Set<number>();
  const bricks: Brick[] = [];
  const brickLookup = new Map<number, number>();
  let id = 1;

  for (const zone of zones) {
    for (let row = 0; row < zone.rows; row += 1) {
      for (let col = 0; col < zone.cols; col += 1) {
        const x = zone.x + col * BRICK_CELL_SIZE;
        const y = zone.y + row * BRICK_CELL_SIZE;
        if (x < area.left || x + BRICK_WIDTH > area.right) continue;
        if (y < area.top || y + BRICK_HEIGHT > area.bottom) continue;

        const cellX = Math.floor(x / BRICK_CELL_SIZE);
        const cellY = Math.floor(y / BRICK_CELL_SIZE);
        const key = buildCellKey(cellX, cellY);
        if (usedCells.has(key)) continue;

        const candidate = { x, y, w: BRICK_WIDTH, h: BRICK_HEIGHT };
        if (walls.some((wall) => isRectOverlap(candidate, wall))) continue;

        usedCells.add(key);
        brickLookup.set(key, bricks.length);
        bricks.push({
          id: id++,
          x,
          y,
          w: BRICK_WIDTH,
          h: BRICK_HEIGHT,
          alive: true,
        });
      }
    }
  }

  bricks.sort((a, b) => (a.y - b.y) || (a.x - b.x));
  brickLookup.clear();
  for (let index = 0; index < bricks.length; index += 1) {
    const brick = bricks[index];
    brick.id = index + 1;
    brickLookup.set(buildCellKey(
      Math.floor(brick.x / BRICK_CELL_SIZE),
      Math.floor(brick.y / BRICK_CELL_SIZE),
    ), index);
  }

  return { bricks, brickLookup };
}

function createLevelLayout(runSeed: number, level: number): LevelLayout {
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const rng = createLevelRng(runSeed ^ Math.imul(attempt + 1, 0x85ebca6b), level);
    const style = LEVEL_LAYOUT_STYLES[randomInt(rng, 0, LEVEL_LAYOUT_STYLES.length - 1)];
    const frame = createFrameSpec(rng, level);
    const area = createBrickArea(frame, rng);
    const walls = layerWalls([
      ...createPerimeterWalls(frame, rng),
      ...createStyleWalls(style, area, rng),
    ]);
    const zones = createBrickZones(style, area, level, rng);
    const built = buildBricksFromZones(zones, walls, area);
    if (built.bricks.length >= MIN_BRICK_COUNT) {
      return { walls, bricks: built.bricks, brickLookup: built.brickLookup };
    }
  }

  const fallbackFrame: FrameSpec = {
    left: 70,
    right: 690,
    top: 90,
    separatorY: 700,
    separatorMode: 'single',
  };
  const fallbackArea: BrickArea = {
    left: 98,
    right: 662,
    top: 140,
    bottom: 670,
  };
  const fallbackRng = createLevelRng(runSeed ^ 0xa53c9e7d, level);
  const fallbackWalls = layerWalls([
    ...createPerimeterWalls(fallbackFrame, fallbackRng),
    ...createStyleWalls('bandsHorizontal', fallbackArea, fallbackRng),
  ]);
  const fallbackBricks = buildBricksFromZones(
    createFallbackZones(fallbackArea),
    fallbackWalls,
    fallbackArea,
  );
  return {
    walls: fallbackWalls,
    bricks: fallbackBricks.bricks,
    brickLookup: fallbackBricks.brickLookup,
  };
}

function createBall(id: number, paddleX: number, paddleWidth: number): Ball {
  const dir = Math.random() > 0.5 ? 1 : -1;
  return {
    id,
    x: paddleX + paddleWidth / 2,
    y: BALL_READY_Y,
    vx: 4.2 * dir,
    vy: -5.2,
    r: BALL_RADIUS,
  };
}

function createReadyBall(id: number, paddleX: number, paddleWidth: number): Ball {
  return {
    id,
    x: paddleX + paddleWidth / 2,
    y: BALL_READY_Y,
    vx: 0,
    vy: 0,
    r: BALL_RADIUS,
  };
}

function createInitialState(runSeed: number = generateRunSeed()): GameState {
  const paddleX = (BOARD_WIDTH - PADDLE_BASE_WIDTH) / 2;
  const layout = createLevelLayout(runSeed, 1);
  return {
    runSeed,
    level: 1,
    levelScore: 0,
    remainingBricks: layout.bricks.length,
    paddleX,
    paddleWidth: PADDLE_BASE_WIDTH,
    isBallLaunched: false,
    expandUntil: 0,
    walls: layout.walls,
    balls: [createReadyBall(1, paddleX, PADDLE_BASE_WIDTH)],
    bricks: layout.bricks,
    brickLookup: layout.brickLookup,
    drops: [],
    dropCounts: createEmptyDropCounts(),
    levelDropCounts: createEmptyDropCounts(),
    score: 0,
    shields: INITIAL_SHIELDS,
    nextBallId: 2,
    nextDropId: 1,
  };
}

function isCircleTouchingRect(ball: Ball, rect: WallRect | Brick): boolean {
  const nearestX = clamp(ball.x, rect.x, rect.x + rect.w);
  const nearestY = clamp(ball.y, rect.y, rect.y + rect.h);
  const dx = ball.x - nearestX;
  const dy = ball.y - nearestY;
  return dx * dx + dy * dy <= ball.r * ball.r;
}

function bounceBallFromRect(ball: Ball, rect: WallRect | Brick) {
  const overlapLeft = ball.x + ball.r - rect.x;
  const overlapRight = rect.x + rect.w - (ball.x - ball.r);
  const overlapTop = ball.y + ball.r - rect.y;
  const overlapBottom = rect.y + rect.h - (ball.y - ball.r);
  const minOverlapX = Math.min(overlapLeft, overlapRight);
  const minOverlapY = Math.min(overlapTop, overlapBottom);

  if (minOverlapX < minOverlapY) {
    if (overlapLeft < overlapRight) {
      ball.x = rect.x - ball.r - 0.5;
      ball.vx = -Math.abs(ball.vx);
    } else {
      ball.x = rect.x + rect.w + ball.r + 0.5;
      ball.vx = Math.abs(ball.vx);
    }
  } else if (overlapTop < overlapBottom) {
    ball.y = rect.y - ball.r - 0.5;
    ball.vy = -Math.abs(ball.vy);
  } else {
    ball.y = rect.y + rect.h + ball.r + 0.5;
    ball.vy = Math.abs(ball.vy);
  }
}

function findFirstCollidingBrick(
  ball: Ball,
  bricks: Brick[],
  brickLookup: Map<number, number>,
): Brick | null {
  const minCellX = Math.floor((ball.x - ball.r) / BRICK_CELL_SIZE) - 1;
  const maxCellX = Math.floor((ball.x + ball.r) / BRICK_CELL_SIZE) + 1;
  const minCellY = Math.floor((ball.y - ball.r) / BRICK_CELL_SIZE) - 1;
  const maxCellY = Math.floor((ball.y + ball.r) / BRICK_CELL_SIZE) + 1;

  for (let cellY = minCellY; cellY <= maxCellY; cellY += 1) {
    for (let cellX = minCellX; cellX <= maxCellX; cellX += 1) {
      const brickIndex = brickLookup.get(buildCellKey(cellX, cellY));
      if (brickIndex == null) continue;
      const brick = bricks[brickIndex];
      if (!brick || !brick.alive) continue;
      if (!isCircleTouchingRect(ball, brick)) continue;
      return brick;
    }
  }
  return null;
}

function resolveBallPairCollision(ballA: Ball, ballB: Ball) {
  let dx = ballB.x - ballA.x;
  let dy = ballB.y - ballA.y;
  let distanceSq = dx * dx + dy * dy;
  const minDistance = ballA.r + ballB.r;
  const minDistanceSq = minDistance * minDistance;

  if (distanceSq > minDistanceSq) return;

  if (distanceSq < 1e-6) {
    const angle = Math.random() * Math.PI * 2;
    dx = Math.cos(angle);
    dy = Math.sin(angle);
    distanceSq = dx * dx + dy * dy;
  }

  const distance = Math.sqrt(distanceSq);
  const nx = dx / distance;
  const ny = dy / distance;

  const overlap = minDistance - distance;
  if (overlap > 0) {
    const correction = overlap * 0.5 + 0.01;
    ballA.x -= nx * correction;
    ballA.y -= ny * correction;
    ballB.x += nx * correction;
    ballB.y += ny * correction;
  }

  const rvx = ballB.vx - ballA.vx;
  const rvy = ballB.vy - ballA.vy;
  const speedAlongNormal = rvx * nx + rvy * ny;
  if (speedAlongNormal >= 0) return;

  const impulse = -((1 + BALL_COLLISION_RESTITUTION) * speedAlongNormal) / 2;
  const impulseX = impulse * nx;
  const impulseY = impulse * ny;
  ballA.vx -= impulseX;
  ballA.vy -= impulseY;
  ballB.vx += impulseX;
  ballB.vy += impulseY;
}

function resolveBallCollisions(balls: Ball[]) {
  if (balls.length < 2) return;
  const spatialGrid = new Map<number, number[]>();

  for (let index = 0; index < balls.length; index += 1) {
    const ball = balls[index];
    const cellX = Math.floor(ball.x / BALL_COLLISION_CELL_SIZE);
    const cellY = Math.floor(ball.y / BALL_COLLISION_CELL_SIZE);

    for (let offsetY = -1; offsetY <= 1; offsetY += 1) {
      for (let offsetX = -1; offsetX <= 1; offsetX += 1) {
        const key = buildBallGridKey(cellX + offsetX, cellY + offsetY);
        const bucket = spatialGrid.get(key);
        if (!bucket) continue;
        for (const otherIndex of bucket) {
          resolveBallPairCollision(ball, balls[otherIndex]);
        }
      }
    }

    const ownKey = buildBallGridKey(cellX, cellY);
    const ownBucket = spatialGrid.get(ownKey);
    if (ownBucket) {
      ownBucket.push(index);
    } else {
      spatialGrid.set(ownKey, [index]);
    }
  }
}

function randomDropType(): DropType {
  const roll = Math.random() * DROP_CHANCE;
  if (roll < DROP_RATE_SPLIT) return 'split';
  if (roll < DROP_RATE_SPLIT + DROP_RATE_TRIPLE) return 'triple';
  if (roll < DROP_RATE_SPLIT + DROP_RATE_TRIPLE + DROP_RATE_EXPAND) return 'expand';
  return 'shield';
}

function drawPixelWall(ctx: CanvasRenderingContext2D, wall: WallRect) {
  ctx.fillStyle = '#b5c2d5';
  ctx.beginPath();
  const dotR = BRICK_WIDTH / 2;
  const dotInset = dotR;
  const dotStep = BRICK_CELL_SIZE;
  const colCount = Math.max(1, Math.floor((wall.w - BRICK_WIDTH) / dotStep) + 1);
  const rowCount = Math.max(1, Math.floor((wall.h - BRICK_HEIGHT) / dotStep) + 1);

  for (let row = 0; row < rowCount; row += 1) {
    const y = wall.y + dotInset + row * dotStep;
    for (let col = 0; col < colCount; col += 1) {
      const x = wall.x + dotInset + col * dotStep;
      ctx.moveTo(x + dotR, y);
      ctx.arc(x, y, dotR, 0, Math.PI * 2);
    }
  }
  ctx.fill();
}

function calcStageSize(viewportWidth: number, viewportHeight: number): StageSize {
  const availableWidth = Math.max(
    280,
    viewportWidth - OVERLAY_GUTTER * 2 - MODAL_INNER_PADDING,
  );
  const availableHeight = Math.max(
    420,
    viewportHeight - OVERLAY_GUTTER * 2 - MODAL_INNER_PADDING - MODAL_EXTRA_HEIGHT,
  );
  const scale = Math.min(availableWidth / BOARD_WIDTH, availableHeight / BOARD_HEIGHT, 1);
  return {
    width: Math.round(BOARD_WIDTH * scale),
    height: Math.round(BOARD_HEIGHT * scale),
  };
}

function isGameEndReason(value: unknown): value is GameEndReason {
  return value === 'gameOver' || value === 'manualExit';
}

function normalizeHistoryRecord(value: unknown): GameHistoryRecord | null {
  if (!value || typeof value !== 'object') return null;
  const candidate = value as Partial<GameHistoryRecord>;
  if (typeof candidate.id !== 'string') return null;
  if (typeof candidate.score !== 'number' || !Number.isFinite(candidate.score)) return null;
  if (typeof candidate.level !== 'number' || !Number.isFinite(candidate.level)) return null;
  if (typeof candidate.durationMs !== 'number' || !Number.isFinite(candidate.durationMs)) return null;
  if (typeof candidate.createdAt !== 'string') return null;
  if (!isGameEndReason(candidate.reason)) return null;
  if (typeof candidate.runSeed !== 'number' || !Number.isFinite(candidate.runSeed)) return null;
  return {
    id: candidate.id,
    score: Math.max(0, Math.floor(candidate.score)),
    level: Math.max(1, Math.floor(candidate.level)),
    durationMs: Math.max(0, Math.floor(candidate.durationMs)),
    createdAt: candidate.createdAt,
    reason: candidate.reason,
    runSeed: Math.floor(candidate.runSeed),
    dropCounts: normalizeDropCounts(candidate.dropCounts),
  };
}

function loadBreakoutHistoryRecords(): GameHistoryRecord[] {
  try {
    const raw = localStorage.getItem(BREAKOUT_HISTORY_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map((item) => normalizeHistoryRecord(item))
      .filter((item): item is GameHistoryRecord => item != null)
      .filter((item) => item.score > 0)
      .sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))
      .slice(0, BREAKOUT_HISTORY_LIMIT);
  } catch {
    return [];
  }
}

function saveBreakoutHistoryRecords(records: GameHistoryRecord[]) {
  try {
    localStorage.setItem(BREAKOUT_HISTORY_STORAGE_KEY, JSON.stringify(records));
  } catch {
    // ignore localStorage write failure
  }
}

function formatHistoryDuration(durationMs: number): string {
  const totalSeconds = Math.floor(Math.max(0, durationMs) / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
}

function formatHistoryTime(createdAt: string): string {
  const timestamp = Date.parse(createdAt);
  if (!Number.isFinite(timestamp)) return '--';
  const date = new Date(timestamp);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  const hour = String(date.getHours()).padStart(2, '0');
  const minute = String(date.getMinutes()).padStart(2, '0');
  return `${year}-${month}-${day} ${hour}:${minute}`;
}

function compareHistoryRecord(a: GameHistoryRecord, b: GameHistoryRecord): number {
  if (b.score !== a.score) return b.score - a.score;
  if (b.level !== a.level) return b.level - a.level;
  if (a.durationMs !== b.durationMs) return a.durationMs - b.durationMs;
  return Date.parse(a.createdAt) - Date.parse(b.createdAt);
}

function getHistoryRank(records: GameHistoryRecord[], targetId: string): number | null {
  const sorted = [...records].sort(compareHistoryRecord);
  const index = sorted.findIndex((item) => item.id === targetId);
  return index >= 0 ? index + 1 : null;
}

export function BreakoutModal({ open, onMinimize, onTerminate }: BreakoutModalProps) {
  const { t } = useTranslation();
  useEscClose(open, onTerminate);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const keysRef = useRef({ left: false, right: false });
  const isStartedRef = useRef(false);
  const isGameOverRef = useRef(false);
  const isLevelClearedRef = useRef(false);
  const isPausedRef = useRef(false);
  const sessionStartedAtRef = useRef<number>(Date.now());
  const sessionRecordedRef = useRef(false);
  const stateRef = useRef<GameState>(createInitialState());
  const lastFrameRef = useRef<number>(0);
  const lastUiSyncRef = useRef<number>(0);
  const rafRef = useRef<number | null>(null);
  const [drops, setDrops] = useState<DropViewModel[]>([]);
  const [level, setLevel] = useState(1);
  const [score, setScore] = useState(0);
  const [shields, setShields] = useState(() => stateRef.current.shields);
  const [isGameOver, setIsGameOver] = useState(false);
  const [isLevelCleared, setIsLevelCleared] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
  const [isStarted, setIsStarted] = useState(false);
  const [isBallLaunched, setIsBallLaunched] = useState(false);
  const [historyRecords, setHistoryRecords] = useState<GameHistoryRecord[]>(() =>
    loadBreakoutHistoryRecords(),
  );
  const [dropIconMap] = useState<DropIconMap>(() => loadBreakoutDropIconMap());
  const [dropTypeOrder] = useState<DropType[]>(() => loadDropTypeOrderSnapshot(dropIconMap));
  const historyRecordsRef = useRef<GameHistoryRecord[]>(historyRecords);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [gameOverRank, setGameOverRank] = useState<number | null>(null);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const [stageSize, setStageSize] = useState<StageSize>(() =>
    calcStageSize(window.innerWidth, window.innerHeight),
  );

  const stageScaleX = stageSize.width / BOARD_WIDTH;
  const stageScaleY = stageSize.height / BOARD_HEIGHT;
  const dropScale = Math.min(stageScaleX, stageScaleY);
  const dropSize = Math.round(clamp(28 * dropScale, 16, 28));
  const dropIconSize = Math.round(clamp(14 * dropScale, 10, 14));
  const sortedRankRecords = useMemo(() => [...historyRecords].sort(compareHistoryRecord), [historyRecords]);
  const topThreeRankRecords = useMemo(() => sortedRankRecords.slice(0, 3), [sortedRankRecords]);
  const latestRecord = historyRecords[0] ?? null;
  const rankingLabel = t('breakout.ranking', {
    defaultValue: t('breakout.history', '历史记录'),
  });
  const rankingShortLabel = t('breakout.rankingShort', {
    defaultValue: t('breakout.historyShort', '历史'),
  });
  const rankingEmptyLabel = t('breakout.rankingEmpty', {
    defaultValue: t('breakout.historyEmpty', '暂无历史记录'),
  });
  const rankingClearLabel = t('breakout.rankingClear', {
    defaultValue: t('breakout.historyClear', '清空'),
  });
  const latestRecordRank = useMemo(() => {
    if (!latestRecord) return null;
    const index = sortedRankRecords.findIndex((record) => record.id === latestRecord.id);
    return index >= 0 ? index + 1 : null;
  }, [latestRecord, sortedRankRecords]);

  useEffect(() => {
    const updateStageSize = () => {
      setStageSize(calcStageSize(window.innerWidth, window.innerHeight));
    };
    updateStageSize();
    window.addEventListener('resize', updateStageSize);
    return () => window.removeEventListener('resize', updateStageSize);
  }, []);

  useEffect(() => {
    historyRecordsRef.current = historyRecords;
  }, [historyRecords]);

  const appendHistoryRecord = useCallback((state: GameState, reason: GameEndReason): number | null => {
    if (sessionRecordedRef.current) return null;
    if (state.score <= 0) return null;
    const durationMs = Math.max(0, Date.now() - sessionStartedAtRef.current);

    if (reason === 'manualExit' && durationMs < 1200 && state.score <= 0 && state.level <= 1) {
      sessionRecordedRef.current = true;
      return null;
    }

    const record: GameHistoryRecord = {
      id: `${Date.now()}-${Math.floor(Math.random() * 100000)}`,
      score: state.score,
      level: state.level,
      durationMs,
      createdAt: new Date().toISOString(),
      reason,
      runSeed: state.runSeed,
      dropCounts: normalizeDropCounts(state.dropCounts),
    };

    const next = [record, ...historyRecordsRef.current].slice(0, BREAKOUT_HISTORY_LIMIT);
    historyRecordsRef.current = next;
    setHistoryRecords(next);
    saveBreakoutHistoryRecords(next);
    sessionRecordedRef.current = true;
    return getHistoryRank(next, record.id);
  }, []);

  const handleClearHistory = useCallback(() => {
    setHistoryRecords([]);
    historyRecordsRef.current = [];
    saveBreakoutHistoryRecords([]);
  }, []);

  const handleEndAndClose = useCallback(() => {
    appendHistoryRecord(stateRef.current, 'manualExit');
    setShowCloseConfirm(false);
    setHistoryOpen(false);
    onTerminate();
  }, [appendHistoryRecord, onTerminate]);

  const handleMinimize = useCallback(() => {
    if (isStartedRef.current && !isGameOverRef.current && !isPausedRef.current) {
      isPausedRef.current = true;
      setIsPaused(true);
    }
    keysRef.current.left = false;
    keysRef.current.right = false;
    setShowCloseConfirm(false);
    setHistoryOpen(false);
    onMinimize();
  }, [onMinimize]);

  const handleRequestClose = useCallback(() => {
    if (isStartedRef.current && !isGameOverRef.current) {
      if (!isPausedRef.current) {
        isPausedRef.current = true;
        setIsPaused(true);
      }
      keysRef.current.left = false;
      keysRef.current.right = false;
      setShowCloseConfirm(true);
      return;
    }
    handleEndAndClose();
  }, [handleEndAndClose]);

  const drawFrame = useCallback((ctx: CanvasRenderingContext2D, state: GameState) => {
    ctx.clearRect(0, 0, BOARD_WIDTH, BOARD_HEIGHT);
    ctx.fillStyle = '#0b1274';
    ctx.fillRect(0, 0, BOARD_WIDTH, BOARD_HEIGHT);

    for (const wall of state.walls) {
      drawPixelWall(ctx, wall);
    }

    ctx.fillStyle = '#2fff71';
    ctx.beginPath();
    for (const brick of state.bricks) {
      if (!brick.alive) continue;
      const cx = brick.x + brick.w / 2;
      const cy = brick.y + brick.h / 2;
      ctx.moveTo(cx + BRICK_RADIUS, cy);
      ctx.arc(cx, cy, BRICK_RADIUS, 0, Math.PI * 2);
    }
    ctx.fill();

    if (state.shields > 0) {
      ctx.fillStyle = 'rgba(188, 227, 255, 0.7)';
      ctx.fillRect(90, BOARD_HEIGHT - 36, BOARD_WIDTH - 180, 4);
    }

    ctx.fillStyle = '#f7fafc';
    ctx.fillRect(state.paddleX, PADDLE_Y, state.paddleWidth, PADDLE_HEIGHT);

    ctx.fillStyle = '#ffffff';
    for (const ball of state.balls) {
      ctx.beginPath();
      ctx.arc(ball.x, ball.y, ball.r, 0, Math.PI * 2);
      ctx.fill();
    }
  }, []);

  const setupLevel = useCallback((state: GameState, levelNumber: number) => {
    const layout = createLevelLayout(state.runSeed, levelNumber);
    const paddleX = (BOARD_WIDTH - PADDLE_BASE_WIDTH) / 2;
    state.level = levelNumber;
    state.levelScore = 0;
    state.remainingBricks = layout.bricks.length;
    state.paddleX = paddleX;
    state.paddleWidth = PADDLE_BASE_WIDTH;
    state.isBallLaunched = false;
    state.expandUntil = 0;
    state.walls = layout.walls;
    state.balls = [createReadyBall(state.nextBallId++, paddleX, PADDLE_BASE_WIDTH)];
    state.bricks = layout.bricks;
    state.brickLookup = layout.brickLookup;
    state.drops = [];
    state.levelDropCounts = createEmptyDropCounts();
  }, []);

  const handleRestart = useCallback(() => {
    const state = createInitialState();
    stateRef.current = state;
    setDrops([]);
    setLevel(state.level);
    setScore(state.score);
    setShields(state.shields);
    setIsGameOver(false);
    setIsLevelCleared(false);
    setIsPaused(false);
    setIsStarted(true);
    setIsBallLaunched(false);
    isStartedRef.current = true;
    isGameOverRef.current = false;
    isLevelClearedRef.current = false;
    isPausedRef.current = false;
    keysRef.current.left = false;
    keysRef.current.right = false;
    lastFrameRef.current = 0;
    sessionStartedAtRef.current = Date.now();
    sessionRecordedRef.current = false;
    setGameOverRank(null);
    setHistoryOpen(false);
  }, []);

  const handleNextLevel = useCallback(() => {
    const state = stateRef.current;
    const nextLevel = state.level + 1;
    setupLevel(state, nextLevel);
    setDrops([]);
    setLevel(nextLevel);
    setScore(state.score);
    setShields(state.shields);
    setIsLevelCleared(false);
    setIsPaused(false);
    setIsStarted(true);
    setIsBallLaunched(false);
    isStartedRef.current = true;
    isLevelClearedRef.current = false;
    isPausedRef.current = false;
    keysRef.current.left = false;
    keysRef.current.right = false;
    lastFrameRef.current = 0;
    setGameOverRank(null);
  }, [setupLevel]);

  const handleStartGame = useCallback(() => {
    if (isStartedRef.current) return;
    isStartedRef.current = true;
    setIsStarted(true);
    setIsBallLaunched(false);
    setHistoryOpen(false);
    setIsPaused(false);
    isPausedRef.current = false;
    keysRef.current.left = false;
    keysRef.current.right = false;
    sessionStartedAtRef.current = Date.now();
    lastFrameRef.current = 0;
  }, []);

  const handleLaunchBall = useCallback(() => {
    if (!isStartedRef.current || isGameOverRef.current || isLevelClearedRef.current || isPausedRef.current) return;
    const state = stateRef.current;
    if (state.isBallLaunched) return;

    if (state.balls.length === 0) {
      state.balls.push(createReadyBall(state.nextBallId++, state.paddleX, state.paddleWidth));
    }

    const ball = state.balls[0];
    if (!ball) return;
    ball.x = state.paddleX + state.paddleWidth / 2;
    ball.y = BALL_READY_Y;
    const moving = createBall(0, state.paddleX, state.paddleWidth);
    ball.vx = moving.vx;
    ball.vy = moving.vy;

    state.isBallLaunched = true;
    setIsBallLaunched(true);
    keysRef.current.left = false;
    keysRef.current.right = false;
    lastFrameRef.current = 0;
  }, []);

  const handlePauseToggle = useCallback(() => {
    if (!isStartedRef.current || isGameOverRef.current || isLevelClearedRef.current) return;
    const nextPaused = !isPausedRef.current;
    isPausedRef.current = nextPaused;
    setIsPaused(nextPaused);
    keysRef.current.left = false;
    keysRef.current.right = false;
    lastFrameRef.current = 0;
  }, []);

  useEffect(() => {
    if (open) return;
    keysRef.current.left = false;
    keysRef.current.right = false;
    setHistoryOpen(false);
    setShowCloseConfirm(false);
    lastFrameRef.current = 0;
  }, [open]);

  const updateGame = useCallback((state: GameState, dt: number, now: number): 'running' | 'gameOver' | 'levelCleared' => {
    if (now > state.expandUntil && state.paddleWidth !== PADDLE_BASE_WIDTH) {
      state.paddleWidth = PADDLE_BASE_WIDTH;
      state.paddleX = clamp(state.paddleX, 0, BOARD_WIDTH - state.paddleWidth);
    }

    if (!state.isBallLaunched) {
      const readyBall = state.balls[0];
      if (readyBall) {
        readyBall.x = state.paddleX + state.paddleWidth / 2;
        readyBall.y = BALL_READY_Y;
        readyBall.vx = 0;
        readyBall.vy = 0;
      }
      return 'running';
    }

    if (keysRef.current.left && !keysRef.current.right) {
      state.paddleX -= PADDLE_SPEED * dt;
    } else if (keysRef.current.right && !keysRef.current.left) {
      state.paddleX += PADDLE_SPEED * dt;
    }
    state.paddleX = clamp(state.paddleX, 0, BOARD_WIDTH - state.paddleWidth);

    const nextBalls: Ball[] = [];
    const paddleRect = { x: state.paddleX, y: PADDLE_Y, w: state.paddleWidth, h: PADDLE_HEIGHT };

    for (const ball of state.balls) {
      const distanceX = ball.vx * dt;
      const distanceY = ball.vy * dt;
      const maxDistance = Math.max(Math.abs(distanceX), Math.abs(distanceY));
      const steps = Math.max(1, Math.ceil(maxDistance / BALL_STEP_DISTANCE));
      const stepX = distanceX / steps;
      const stepY = distanceY / steps;
      let lost = false;

      for (let step = 0; step < steps; step += 1) {
        ball.x += stepX;
        ball.y += stepY;

        if (ball.x - ball.r <= 0) {
          ball.x = ball.r + 0.5;
          ball.vx = Math.abs(ball.vx);
        }
        if (ball.x + ball.r >= BOARD_WIDTH) {
          ball.x = BOARD_WIDTH - ball.r - 0.5;
          ball.vx = -Math.abs(ball.vx);
        }
        if (ball.y - ball.r <= 0) {
          ball.y = ball.r + 0.5;
          ball.vy = Math.abs(ball.vy);
        }

        for (let resolve = 0; resolve < 4; resolve += 1) {
          let collided = false;
          for (const wall of state.walls) {
            if (!isCircleTouchingRect(ball, wall)) continue;
            bounceBallFromRect(ball, wall);
            collided = true;
          }
          if (!collided) break;
        }

        if (ball.vy > 0 && isCircleTouchingRect(ball, paddleRect)) {
          const center = state.paddleX + state.paddleWidth / 2;
          const ratio = (ball.x - center) / Math.max(1, state.paddleWidth / 2);
          ball.y = PADDLE_Y - ball.r - 0.5;
          ball.vx = clamp(ball.vx + ratio * 2.6, -10, 10);
          ball.vy = -Math.max(4.8, 5.2 + Math.abs(ratio) * 1.8);
        }

        const hitBrick = findFirstCollidingBrick(ball, state.bricks, state.brickLookup);
        if (hitBrick) {
          hitBrick.alive = false;
          state.remainingBricks = Math.max(0, state.remainingBricks - 1);
          state.score += 1;
          state.levelScore += 1;
          bounceBallFromRect(ball, hitBrick);

          if (state.drops.length < MAX_DROPS && Math.random() < DROP_CHANCE) {
            state.drops.push({
              id: state.nextDropId++,
              type: randomDropType(),
              x: hitBrick.x + hitBrick.w / 2,
              y: hitBrick.y + hitBrick.h / 2,
              vy: DROP_SPEED,
            });
          }
        }

        if (ball.y - ball.r > BOARD_HEIGHT + 28) {
          lost = true;
          break;
        }
      }

      if (!lost) {
        nextBalls.push(ball);
      }
    }
    state.balls = nextBalls;

    const nextDrops: DropItem[] = [];
    for (const drop of state.drops) {
      drop.y += drop.vy * dt;
      const hitPaddle =
        drop.x >= state.paddleX &&
        drop.x <= state.paddleX + state.paddleWidth &&
        drop.y >= PADDLE_Y - 10 &&
        drop.y <= PADDLE_Y + PADDLE_HEIGHT + 10;

      if (hitPaddle) {
        state.dropCounts[drop.type] += 1;
        state.levelDropCounts[drop.type] += 1;
        if (drop.type === 'split') {
          const freeSlots = Math.max(0, MAX_BALLS - state.balls.length);
          const spawnCount = Math.min(3, freeSlots);
          if (spawnCount > 0) {
            const paddleCenterX = state.paddleX + state.paddleWidth / 2;
            const spawnY = BALL_READY_Y;
            const spread = 8;
            let shotConfigs: Array<{ offsetX: number; vx: number }> = [];
            if (spawnCount === 1) {
              shotConfigs = [{ offsetX: 0, vx: 0 }];
            } else if (spawnCount === 2) {
              shotConfigs = [
                { offsetX: -spread, vx: -SPLIT_SHOT_SIDE_DELTA_VX },
                { offsetX: spread, vx: SPLIT_SHOT_SIDE_DELTA_VX },
              ];
            } else {
              shotConfigs = [
                { offsetX: -spread, vx: -SPLIT_SHOT_SIDE_DELTA_VX },
                { offsetX: 0, vx: 0 },
                { offsetX: spread, vx: SPLIT_SHOT_SIDE_DELTA_VX },
              ];
            }

            const spawnedBalls = shotConfigs.map((config) => ({
              id: state.nextBallId++,
              x: clamp(
                paddleCenterX + config.offsetX,
                BALL_RADIUS + 0.5,
                BOARD_WIDTH - BALL_RADIUS - 0.5,
              ),
              y: spawnY,
              vx: config.vx,
              vy: -SPLIT_SHOT_UP_SPEED,
              r: BALL_RADIUS,
            }));
            state.balls.push(...spawnedBalls);
            state.isBallLaunched = true;
          }
        } else if (drop.type === 'triple') {
          if (state.balls.length === 0) {
            state.balls.push(createBall(state.nextBallId++, state.paddleX, state.paddleWidth));
          } else {
            const sourceBalls = state.balls.slice();
            const spawnedBalls: Ball[] = [];

            for (const source of sourceBalls) {
              source.vy = -Math.max(TRIPLE_SHOT_MIN_UP_SPEED, Math.abs(source.vy));

              const freeSlots = MAX_BALLS - (state.balls.length + spawnedBalls.length);
              if (freeSlots <= 0) break;

              const sideVxList = [
                clamp(source.vx - TRIPLE_SHOT_SIDE_DELTA_VX, -10, 10),
                clamp(source.vx + TRIPLE_SHOT_SIDE_DELTA_VX, -10, 10),
              ];

              for (let index = 0; index < sideVxList.length; index += 1) {
                const freeSlotsInner = MAX_BALLS - (state.balls.length + spawnedBalls.length);
                if (freeSlotsInner <= 0) break;
                spawnedBalls.push({
                  id: state.nextBallId++,
                  x: source.x + (index === 0 ? -2 : 2),
                  y: source.y - 2,
                  vx: sideVxList[index],
                  vy: -Math.max(TRIPLE_SHOT_MIN_UP_SPEED, Math.abs(source.vy)),
                  r: source.r,
                });
              }
            }

            if (spawnedBalls.length > 0) {
              state.balls.push(...spawnedBalls);
            }
          }
        } else if (drop.type === 'expand') {
          state.paddleWidth = Math.min(PADDLE_EXPAND_MAX_WIDTH, state.paddleWidth + 40);
          state.expandUntil = now + PADDLE_EXPAND_DURATION_MS;
          state.paddleX = clamp(state.paddleX, 0, BOARD_WIDTH - state.paddleWidth);
        } else if (drop.type === 'shield') {
          state.shields = Math.min(3, state.shields + 1);
        }
        continue;
      }

      if (drop.y <= BOARD_HEIGHT + 28) {
        nextDrops.push(drop);
      }
    }
    state.drops = nextDrops;
    resolveBallCollisions(state.balls);

    if (state.balls.length === 0) {
      if (state.shields > 0) {
        state.shields -= 1;
        state.balls.push(createBall(state.nextBallId++, state.paddleX, state.paddleWidth));
        state.isBallLaunched = true;
      } else {
        return 'gameOver';
      }
    }

    if (state.remainingBricks <= 0) {
      state.drops = [];
      return 'levelCleared';
    }
    return 'running';
  }, []);

  const updateLevelClearedBackground = useCallback((state: GameState, dt: number) => {
    for (const ball of state.balls) {
      const distanceX = ball.vx * dt;
      const distanceY = ball.vy * dt;
      const maxDistance = Math.max(Math.abs(distanceX), Math.abs(distanceY));
      const steps = Math.max(1, Math.ceil(maxDistance / BALL_STEP_DISTANCE));
      const stepX = distanceX / steps;
      const stepY = distanceY / steps;

      for (let step = 0; step < steps; step += 1) {
        ball.x += stepX;
        ball.y += stepY;

        if (ball.x - ball.r <= 0) {
          ball.x = ball.r + 0.5;
          ball.vx = Math.abs(ball.vx);
        }
        if (ball.x + ball.r >= BOARD_WIDTH) {
          ball.x = BOARD_WIDTH - ball.r - 0.5;
          ball.vx = -Math.abs(ball.vx);
        }
        if (ball.y - ball.r <= 0) {
          ball.y = ball.r + 0.5;
          ball.vy = Math.abs(ball.vy);
        }
        if (ball.y + ball.r >= BOARD_HEIGHT) {
          ball.y = BOARD_HEIGHT - ball.r - 0.5;
          ball.vy = -Math.abs(ball.vy);
        }

        for (let resolve = 0; resolve < 4; resolve += 1) {
          let collided = false;
          for (const wall of state.walls) {
            if (!isCircleTouchingRect(ball, wall)) continue;
            bounceBallFromRect(ball, wall);
            collided = true;
          }
          if (!collided) break;
        }
      }
    }
    resolveBallCollisions(state.balls);
  }, []);

  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      if (key === 'escape') {
        handleRequestClose();
        return;
      }

      if ((key === 'enter' || key === ' ') && isGameOverRef.current) {
        event.preventDefault();
        handleRestart();
        return;
      }

      if ((key === 'enter' || key === ' ') && isLevelClearedRef.current) {
        event.preventDefault();
        handleNextLevel();
        return;
      }

      if (key === ' ' && !isStartedRef.current) {
        event.preventDefault();
        handleStartGame();
        return;
      }

      if (key === ' ' && isStartedRef.current && !isGameOverRef.current && !isLevelClearedRef.current) {
        event.preventDefault();
        if (isPausedRef.current) {
          handlePauseToggle();
          return;
        }
        if (!stateRef.current.isBallLaunched) {
          handleLaunchBall();
          return;
        }
        handlePauseToggle();
        return;
      }

      if (key === 'p' && isStartedRef.current && !isGameOverRef.current && !isLevelClearedRef.current) {
        event.preventDefault();
        handlePauseToggle();
        return;
      }

      if (!isStartedRef.current || isGameOverRef.current || isPausedRef.current || isLevelClearedRef.current) {
        return;
      }

      if (!stateRef.current.isBallLaunched) {
        return;
      }

      if (key === 'arrowleft' || key === 'a') {
        keysRef.current.left = true;
        event.preventDefault();
      }
      if (key === 'arrowright' || key === 'd') {
        keysRef.current.right = true;
        event.preventDefault();
      }
    };

    const handleKeyUp = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      if (key === 'arrowleft' || key === 'a') {
        keysRef.current.left = false;
      }
      if (key === 'arrowright' || key === 'd') {
        keysRef.current.right = false;
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
    };
  }, [handleLaunchBall, handleNextLevel, handlePauseToggle, handleRequestClose, handleRestart, handleStartGame, open]);

  useEffect(() => {
    if (!open) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const loop = (timestamp: number) => {
      const state = stateRef.current;
      const lastTs = lastFrameRef.current || timestamp;
      const dt = Math.min((timestamp - lastTs) / 16.667, 2.2);
      lastFrameRef.current = timestamp;

      if (isStartedRef.current && !isGameOverRef.current && !isPausedRef.current && !isLevelClearedRef.current) {
        const tickResult = updateGame(state, dt, timestamp);
        if (tickResult === 'gameOver') {
          const rank = appendHistoryRecord(state, 'gameOver');
          setGameOverRank(rank);
          isGameOverRef.current = true;
          setIsGameOver(true);
        } else if (tickResult === 'levelCleared') {
          isLevelClearedRef.current = true;
          setIsLevelCleared(true);
          keysRef.current.left = false;
          keysRef.current.right = false;
        }
      } else if (isLevelClearedRef.current) {
        updateLevelClearedBackground(state, dt);
      }
      drawFrame(ctx, state);

      if (timestamp - lastUiSyncRef.current > UI_SYNC_INTERVAL_MS) {
        setDrops((prev) => {
          const nextLen = state.drops.length;
          if (nextLen === 0) {
            return prev.length === 0 ? prev : [];
          }

          let changed = prev.length !== nextLen;
          if (!changed) {
            for (let index = 0; index < nextLen; index += 1) {
              const prevDrop = prev[index];
              const nextDrop = state.drops[index];
              if (
                prevDrop.id !== nextDrop.id ||
                prevDrop.type !== nextDrop.type ||
                Math.abs(prevDrop.x - nextDrop.x) > 0.75 ||
                Math.abs(prevDrop.y - nextDrop.y) > 0.75
              ) {
                changed = true;
                break;
              }
            }
          }
          if (!changed) return prev;

          const nextDrops: DropViewModel[] = new Array(nextLen);
          for (let index = 0; index < nextLen; index += 1) {
            const drop = state.drops[index];
            nextDrops[index] = { id: drop.id, type: drop.type, x: drop.x, y: drop.y };
          }
          return nextDrops;
        });
        setLevel((prev) => (prev === state.level ? prev : state.level));
        setScore((prev) => (prev === state.score ? prev : state.score));
        setShields((prev) => (prev === state.shields ? prev : state.shields));
        setIsBallLaunched((prev) => (prev === state.isBallLaunched ? prev : state.isBallLaunched));
        lastUiSyncRef.current = timestamp;
      }

      rafRef.current = window.requestAnimationFrame(loop);
    };

    rafRef.current = window.requestAnimationFrame(loop);
    return () => {
      if (rafRef.current != null) {
        window.cancelAnimationFrame(rafRef.current);
      }
    };
  }, [appendHistoryRecord, drawFrame, open, updateGame, updateLevelClearedBackground]);

  if (!open) {
    return null;
  }

  return (
    <div className="modal-overlay breakout-overlay">
      <div className="breakout-modal" onClick={(event) => event.stopPropagation()}>
        <div
          className="breakout-stage"
          style={{ width: `${stageSize.width}px`, height: `${stageSize.height}px` }}
        >
          <canvas
            ref={canvasRef}
            className="breakout-canvas"
            width={BOARD_WIDTH}
            height={BOARD_HEIGHT}
          />

          <div className="breakout-drop-layer" aria-hidden="true">
            {drops.map((drop) => (
              <div
                key={drop.id}
                className={`breakout-drop breakout-drop-${drop.type}`}
                style={{
                  left: `${drop.x * stageScaleX}px`,
                  top: `${drop.y * stageScaleY}px`,
                  width: `${dropSize}px`,
                  height: `${dropSize}px`,
                }}
              >
                <div className="breakout-drop-icon" style={{ width: `${dropIconSize}px`, height: `${dropIconSize}px` }}>
                  {renderPlatformIcon(dropIconMap[drop.type], dropIconSize)}
                </div>
              </div>
            ))}
          </div>

          <div className="breakout-hud" aria-hidden="true">
            <div className="breakout-hud-left">
              <span className="breakout-score">{score}</span>
              <span className="breakout-level">
                {t('breakout.level', { level, defaultValue: `关卡 ${level}` })}
              </span>
            </div>
            <div className="breakout-hud-actions">
              <button
                type="button"
                className="breakout-history-btn"
                onClick={() => setHistoryOpen((open) => !open)}
                title={rankingLabel}
                aria-label={rankingLabel}
              >
                {rankingShortLabel}
              </button>
              <button
                type="button"
                className="breakout-pause-btn"
                onClick={handlePauseToggle}
                title={isPaused ? t('breakout.resume', '继续') : t('breakout.pause', '暂停')}
                aria-label={isPaused ? t('breakout.resume', '继续') : t('breakout.pause', '暂停')}
              >
                {isPaused ? <Play size={14} /> : <Pause size={14} />}
              </button>
              <button
                type="button"
                className="breakout-window-btn"
                onClick={handleMinimize}
                title={t('breakout.minimize', '最小化')}
                aria-label={t('breakout.minimize', '最小化')}
              >
                <Minus size={14} />
              </button>
              <button
                type="button"
                className="breakout-window-btn close"
                onClick={handleRequestClose}
                title={t('common.close', '关闭')}
                aria-label={t('common.close', '关闭')}
              >
                <X size={14} />
              </button>
              <span className="breakout-shields">
                {Array.from({ length: shields }).map((_, index) => (
                  <span key={`shield-${index}`} className="breakout-shield-dot" />
                ))}
              </span>
            </div>
          </div>

          {historyOpen && (
            <div className="breakout-history-panel">
              <div className="breakout-history-header">
                <div className="breakout-history-title">
                  {rankingLabel}
                </div>
                <div className="breakout-history-header-actions">
                  <button
                    type="button"
                    className="breakout-history-clear"
                    onClick={handleClearHistory}
                  >
                    {rankingClearLabel}
                  </button>
                  <button
                    type="button"
                    className="breakout-history-close"
                    onClick={() => setHistoryOpen(false)}
                    aria-label={t('common.close', '关闭')}
                    title={t('common.close', '关闭')}
                  >
                    <X size={14} />
                  </button>
                </div>
              </div>

              {sortedRankRecords.length === 0 ? (
                <div className="breakout-history-empty">{rankingEmptyLabel}</div>
              ) : (
                <div className="breakout-history-list">
                  {sortedRankRecords.map((record, index) => (
                    <div key={record.id} className="breakout-history-item">
                      <div className="breakout-history-item-head">
                        <span className={`breakout-history-rank${index < 3 ? ' is-top' : ''}`}>
                          {t('breakout.rankingRankShort', {
                            rank: index + 1,
                            defaultValue: `#${index + 1}`,
                          })}
                        </span>
                        <span className="breakout-history-score">
                          {t('breakout.historyScoreShort', { score: record.score, defaultValue: `分 ${record.score}` })}
                        </span>
                      </div>
                      <div className="breakout-history-item-meta">
                        <span>{t('breakout.historyLevelShort', { level: record.level, defaultValue: `关 ${record.level}` })}</span>
                        <span>{t('breakout.historyDurationShort', { duration: formatHistoryDuration(record.durationMs), defaultValue: `时长 ${formatHistoryDuration(record.durationMs)}` })}</span>
                        <span>
                          {record.reason === 'gameOver'
                            ? t('breakout.historyReasonGameOver', '本局结束')
                            : t('breakout.historyReasonManualExit', '手动退出')}
                        </span>
                        <span>{formatHistoryTime(record.createdAt)}</span>
                      </div>
                      <div className="breakout-history-item-drops">
                        <div className="breakout-drop-counts">
                          {getSortedDropTypes(record.dropCounts, dropTypeOrder).map((dropType) => (
                            <span key={`${record.id}-${dropType}`} className="breakout-drop-count-chip">
                              <span className="breakout-drop-count-icon">
                                {renderPlatformIcon(dropIconMap[dropType], 12)}
                              </span>
                              <span className="breakout-drop-count-value">x{record.dropCounts[dropType]}</span>
                            </span>
                          ))}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          {showCloseConfirm && (
            <div className="breakout-close-confirm">
              <div className="breakout-close-confirm-card">
                <div className="breakout-close-confirm-title">
                  {t('breakout.closeConfirmTitle', '结束本局？')}
                </div>
                <div className="breakout-close-confirm-desc">
                  {t('breakout.closeConfirmDesc', '关闭后本局将结束，进度不会保留。')}
                </div>
                <div className="breakout-close-confirm-actions">
                  <button
                    type="button"
                    className="breakout-close-confirm-btn"
                    onClick={() => setShowCloseConfirm(false)}
                  >
                    {t('breakout.closeContinue', '继续游戏')}
                  </button>
                  <button
                    type="button"
                    className="breakout-close-confirm-btn"
                    onClick={handleMinimize}
                  >
                    {t('breakout.closeMinimize', '最小化保留')}
                  </button>
                  <button
                    type="button"
                    className="breakout-close-confirm-btn danger"
                    onClick={handleEndAndClose}
                  >
                    {t('breakout.closeEnd', '结束并关闭')}
                  </button>
                </div>
              </div>
            </div>
          )}

          {!isStarted && !isGameOver && !isLevelCleared && (
            <div className="breakout-start-screen">
              <div className="breakout-start-title">{t('breakout.startGame', '开始游戏')}</div>
              <div className="breakout-start-hint">{t('breakout.startHint', '点击开始按钮或按空格')}</div>
              <div className="breakout-rank-summary">
                <div className="breakout-rank-summary-title">{t('breakout.rankTitle', '历史排名')}</div>
                <div className="breakout-rank-summary-list">
                  {Array.from({ length: 3 }).map((_, index) => {
                    const record = topThreeRankRecords[index];
                    const isCurrent = latestRecordRank != null && latestRecordRank === index + 1;
                    return (
                      <div key={`start-rank-${index}`} className={`breakout-rank-row${isCurrent ? ' is-current' : ''}`}>
                        <span className="breakout-rank-index">#{index + 1}</span>
                        <span className="breakout-rank-value">
                          {record
                            ? t('breakout.historyScoreShort', { score: record.score, defaultValue: `分 ${record.score}` })
                            : t('breakout.rankNoData', '--')}
                        </span>
                        <span className="breakout-rank-tag">{isCurrent ? t('breakout.rankCurrent', '当前') : ''}</span>
                      </div>
                    );
                  })}
                  {latestRecordRank != null && latestRecordRank > 3 && (
                    <div className="breakout-rank-row is-current">
                      <span className="breakout-rank-index">#{latestRecordRank}</span>
                      <span className="breakout-rank-value">
                        {latestRecord
                          ? t('breakout.historyScoreShort', { score: latestRecord.score, defaultValue: `分 ${latestRecord.score}` })
                          : t('breakout.rankNoData', '--')}
                      </span>
                      <span className="breakout-rank-tag">{t('breakout.rankCurrent', '当前')}</span>
                    </div>
                  )}
                </div>
              </div>
              <div className="breakout-start-actions">
                <button className="breakout-restart-btn" onClick={handleStartGame}>
                  {t('breakout.startGame', '开始游戏')}
                </button>
                <button
                  type="button"
                  className="breakout-history-btn breakout-start-history-btn"
                  onClick={() => setHistoryOpen((open) => !open)}
                >
                  {rankingLabel}
                </button>
              </div>
            </div>
          )}

          {isStarted && !isBallLaunched && !isGameOver && !isLevelCleared && !isPaused && (
            <div className="breakout-serve-hint">
              {t('breakout.serveHint', '按空格开始发球')}
            </div>
          )}

          {isPaused && !isGameOver && (
            <div className="breakout-paused">
              <div className="breakout-paused-title">{t('breakout.paused', '已暂停')}</div>
              <button className="breakout-restart-btn" onClick={handlePauseToggle}>
                {t('breakout.resume', '继续')}
              </button>
            </div>
          )}

          {isLevelCleared && !isGameOver && (
            <div className="breakout-level-clear">
              <div className="breakout-level-clear-title">
                {t('breakout.levelCleared', '关卡完成')}
              </div>
              <div className="breakout-level-clear-stats">
                <div className="breakout-level-clear-score">
                  {t('breakout.levelScore', {
                    score: stateRef.current.levelScore,
                    defaultValue: `本关分数 ${stateRef.current.levelScore}`,
                  })}
                </div>
                <div className="breakout-drop-counts breakout-drop-counts-levelclear">
                  {getSortedDropTypes(stateRef.current.levelDropCounts, dropTypeOrder).map((dropType) => (
                    <span key={`levelclear-${dropType}`} className="breakout-drop-count-chip">
                      <span className="breakout-drop-count-icon">
                        {renderPlatformIcon(dropIconMap[dropType], 13)}
                      </span>
                      <span className="breakout-drop-count-value">x{stateRef.current.levelDropCounts[dropType]}</span>
                    </span>
                  ))}
                </div>
              </div>
              <button className="breakout-restart-btn" onClick={handleNextLevel}>
                {t('breakout.nextLevel', '下一关')}
              </button>
            </div>
          )}

          {isGameOver && (
            <div className="breakout-gameover">
              <div className="breakout-gameover-title">{t('breakout.gameOver', '本局结束')}</div>
              <div className="breakout-gameover-drops">
                <div className="breakout-drop-counts breakout-drop-counts-gameover">
                  {getSortedDropTypes(stateRef.current.dropCounts, dropTypeOrder).map((dropType) => (
                    <span key={`gameover-${dropType}`} className="breakout-drop-count-chip">
                      <span className="breakout-drop-count-icon">
                        {renderPlatformIcon(dropIconMap[dropType], 13)}
                      </span>
                      <span className="breakout-drop-count-value">x{stateRef.current.dropCounts[dropType]}</span>
                    </span>
                  ))}
                </div>
              </div>
              <div className="breakout-rank-summary">
                <div className="breakout-rank-summary-title">{t('breakout.rankTitle', '历史排名')}</div>
                <div className="breakout-rank-summary-list">
                  {Array.from({ length: 3 }).map((_, index) => {
                    const record = topThreeRankRecords[index];
                    const isCurrent = gameOverRank != null && gameOverRank === index + 1;
                    return (
                      <div key={`gameover-rank-${index}`} className={`breakout-rank-row${isCurrent ? ' is-current' : ''}`}>
                        <span className="breakout-rank-index">#{index + 1}</span>
                        <span className="breakout-rank-value">
                          {record
                            ? t('breakout.historyScoreShort', { score: record.score, defaultValue: `分 ${record.score}` })
                            : t('breakout.rankNoData', '--')}
                        </span>
                        <span className="breakout-rank-tag">{isCurrent ? t('breakout.rankCurrent', '当前') : ''}</span>
                      </div>
                    );
                  })}
                  {gameOverRank != null && gameOverRank > 3 && (
                    <div className="breakout-rank-row is-current">
                      <span className="breakout-rank-index">#{gameOverRank}</span>
                      <span className="breakout-rank-value">
                        {t('breakout.historyScoreShort', { score, defaultValue: `分 ${score}` })}
                      </span>
                      <span className="breakout-rank-tag">{t('breakout.rankCurrent', '当前')}</span>
                    </div>
                  )}
                </div>
              </div>
              <button className="breakout-restart-btn" onClick={handleRestart}>
                {t('breakout.restart', '重新开始')}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
