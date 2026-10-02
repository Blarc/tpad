"use strict";

import Fuse from "./assets/fuse.basic.min.mjs";

const elements = {
  appShell: document.querySelector(".app-shell"),
  sidebar: document.querySelector("#sidebar"),
  sidebarResizer: document.querySelector("#sidebar-resizer"),
  closeSidebar: document.querySelector("#close-sidebar"),
  openSidebar: document.querySelector("#open-sidebar"),
  rootActions: document.querySelector("#root-actions"),
  rootRow: document.querySelector("#root-row"),
  browserError: document.querySelector("#browser-error"),
  entryList: document.querySelector("#entry-list"),
  treeEntries: document.querySelector("#tree-entries"),
  rootMarker: document.querySelector("#root-marker"),
  filename: document.querySelector("#current-filename"),
  saveStatus: document.querySelector("#save-status"),
  editor: document.querySelector("#editor"),
  findTree: document.querySelector("#find-tree"),
  treeSearchDialog: document.querySelector("#tree-search-dialog"),
  closeTreeSearch: document.querySelector("#close-tree-search"),
  treeSearchInput: document.querySelector("#tree-search-input"),
  treeSearchStatus: document.querySelector("#tree-search-status"),
  treeSearchResults: document.querySelector("#tree-search-results"),
  actionDialog: document.querySelector("#action-dialog"),
  actionForm: document.querySelector("#action-form"),
  actionTitle: document.querySelector("#action-title"),
  actionLocation: document.querySelector("#action-location"),
  actionLabel: document.querySelector("#action-label"),
  actionName: document.querySelector("#action-name"),
  actionMessage: document.querySelector("#action-message"),
  actionError: document.querySelector("#action-error"),
  actionSubmit: document.querySelector("#action-submit"),
  entryMenu: document.querySelector("#entry-menu"),
  newFileInFolder: document.querySelector("#new-file-in-folder"),
  newFolderInFolder: document.querySelector("#new-folder-in-folder"),
  renameEntry: document.querySelector("#rename-entry"),
  deleteEntry: document.querySelector("#delete-entry"),
};

const state = {
  sidebarWidth: 240,
  resizePointerId: null,
  resizeStartX: 0,
  resizeStartWidth: 240,
  directory: "",
  entriesByDirectory: new Map(),
  expandedDirectories: new Set([""]),
  loadingDirectories: new Set(),
  currentFile: null,
  selectedPath: null,
  contextEntry: null,
  contextTarget: null,
  actionKind: "create-file",
  actionDirectory: "",
  actionEntry: null,
  actionReturnFocus: null,
  rawText: "",
  normalizedText: "",
  newline: "\n",
  editVersion: 0,
  dirty: false,
  savePromise: null,
  saveTimer: null,
  lastSaveOk: true,
  treeSearchFuse: null,
  treeSearchEntries: [],
  treeSearchMatches: [],
  treeSearchIndex: -1,
  treeSearchReturnFocus: null,
  treeSearchLoadId: 0,
};

const MIN_SIDEBAR_WIDTH = 150;
const MAX_SIDEBAR_WIDTH = 480;
const MIN_EDITOR_WIDTH = 200;
const RESIZER_WIDTH = 9;
const MAX_TREE_SEARCH_RESULTS = 50;

function setSidebarWidth(width) {
  const maximum = Math.min(
    MAX_SIDEBAR_WIDTH,
    window.innerWidth - MIN_EDITOR_WIDTH - RESIZER_WIDTH,
  );
  state.sidebarWidth = Math.round(Math.max(MIN_SIDEBAR_WIDTH, Math.min(width, maximum)));
  elements.appShell.style.setProperty("--sidebar-width", `${state.sidebarWidth}px`);
  elements.sidebarResizer.setAttribute("aria-valuenow", String(state.sidebarWidth));
}

function stopSidebarResize() {
  if (state.resizePointerId === null) return;
  state.resizePointerId = null;
  document.body.classList.remove("resizing-sidebar");
}

elements.sidebarResizer.addEventListener("pointerdown", (event) => {
  if (!event.isPrimary || event.button !== 0 || window.matchMedia("(max-width: 700px)").matches) return;
  event.preventDefault();
  state.resizePointerId = event.pointerId;
  state.resizeStartX = event.clientX;
  state.resizeStartWidth = state.sidebarWidth;
  elements.sidebarResizer.setPointerCapture(event.pointerId);
  document.body.classList.add("resizing-sidebar");
});

elements.sidebarResizer.addEventListener("pointermove", (event) => {
  if (event.pointerId !== state.resizePointerId) return;
  setSidebarWidth(state.resizeStartWidth + event.clientX - state.resizeStartX);
});

elements.sidebarResizer.addEventListener("pointerup", stopSidebarResize);
elements.sidebarResizer.addEventListener("pointercancel", stopSidebarResize);
elements.sidebarResizer.addEventListener("lostpointercapture", stopSidebarResize);

elements.sidebarResizer.addEventListener("keydown", (event) => {
  if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
    event.preventDefault();
    setSidebarWidth(state.sidebarWidth + (event.key === "ArrowLeft" ? -10 : 10));
  }
});

function apiUrl(route, path) {
  const url = new URL(route, window.location.href);
  if (path !== undefined) url.searchParams.set("path", path);
  return url;
}

async function apiRequest(route, options = {}, path) {
  const response = await fetch(apiUrl(route, path), {
    cache: "no-store",
    ...options,
  });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = await response.json();
      if (body.error?.message) message = body.error.message;
    } catch (_) {
      // Keep the status text when the response is not JSON.
    }
    throw new Error(message);
  }
  return response;
}

function joinPath(directory, name) {
  return directory ? `${directory}/${name}` : name;
}

function parentPath(path) {
  const separator = path.lastIndexOf("/");
  return separator === -1 ? "" : path.slice(0, separator);
}

function basename(path) {
  const separator = path.lastIndexOf("/");
  return separator === -1 ? path : path.slice(separator + 1);
}

function rebasePath(path, oldPrefix, newPrefix) {
  if (path === oldPrefix) return newPrefix;
  return path.startsWith(`${oldPrefix}/`)
    ? `${newPrefix}${path.slice(oldPrefix.length)}`
    : path;
}

function showError(element, error) {
  element.textContent = error instanceof Error ? error.message : String(error);
  element.hidden = false;
}

function clearError(element) {
  element.textContent = "";
  element.hidden = true;
}

function createEntryIcon(name) {
  const icon = document.createElement("picture");
  icon.classList.add("entry-icon");
  icon.setAttribute("aria-hidden", "true");

  const darkSource = document.createElement("source");
  darkSource.srcset = `./assets/${name}-dark.svg`;
  darkSource.media = "(prefers-color-scheme: dark)";
  icon.append(darkSource);

  const image = document.createElement("img");
  image.src = `./assets/${name}.svg`;
  image.alt = "";
  image.width = 16;
  image.height = 16;
  image.draggable = false;
  icon.append(image);
  return icon;
}

function createChevronIcon(expanded) {
  const direction = expanded ? "down" : "right";
  const icon = document.createElement("picture");
  icon.classList.add("tree-chevron");

  const darkSource = document.createElement("source");
  darkSource.srcset = `./assets/chevron-${direction}-dark.svg`;
  darkSource.media = "(prefers-color-scheme: dark)";
  icon.append(darkSource);

  const image = document.createElement("img");
  image.src = `./assets/chevron-${direction}.svg`;
  image.alt = "";
  image.width = 16;
  image.height = 16;
  image.draggable = false;
  icon.append(image);
  return icon;
}

function setSaveStatus(label, failed = false) {
  elements.saveStatus.textContent = label;
  elements.saveStatus.classList.toggle("failed", failed);
}

function closeMobileSidebar() {
  elements.sidebar.classList.remove("open");
  elements.openSidebar.setAttribute("aria-expanded", "false");
}

function openMobileSidebar() {
  elements.sidebar.classList.add("open");
  elements.openSidebar.setAttribute("aria-expanded", "true");
  const target = elements.entryList.querySelector("button:not(:disabled)") || elements.rootActions;
  if (target === elements.rootActions) focusTreeItem("");
  else focusTreeItem(target.dataset.path);
}

async function fetchDirectory(path) {
  const response = await apiRequest("api/entries", {}, path);
  const listing = await response.json();
  state.entriesByDirectory.set(path, listing.entries);
}

function updateTreeSearchResults() {
  elements.treeSearchResults.replaceChildren();
  elements.treeSearchInput.removeAttribute("aria-activedescendant");
  state.treeSearchMatches = [];
  state.treeSearchIndex = -1;

  if (!state.treeSearchFuse) return;
  const query = elements.treeSearchInput.value;
  if (query.length === 0) {
    elements.treeSearchStatus.textContent = "Type to search files and folders";
    return;
  }

  const matches = state.treeSearchFuse.search(query);
  state.treeSearchMatches = matches
    .slice(0, MAX_TREE_SEARCH_RESULTS)
    .map((match) => match.item);
  if (state.treeSearchMatches.length === 0) {
    elements.treeSearchStatus.textContent = "No matches";
    return;
  }

  const fragment = document.createDocumentFragment();
  state.treeSearchMatches.forEach((entry, index) => {
    const option = document.createElement("div");
    option.id = `tree-search-option-${index}`;
    option.className = "tree-search-option";
    option.setAttribute("role", "option");
    option.setAttribute("aria-selected", "false");
    option.textContent = `${entry.path}${entry.kind === "directory" ? "/" : ""}`;
    option.addEventListener("click", () => chooseTreeSearchResult(index));
    fragment.append(option);
  });
  elements.treeSearchResults.replaceChildren(fragment);
  const suffix = matches.length > MAX_TREE_SEARCH_RESULTS ? "; first 50 shown" : "";
  elements.treeSearchStatus.textContent = `${matches.length} match${matches.length === 1 ? "" : "es"}${suffix}`;
  setTreeSearchSelection(0);
}

function setTreeSearchSelection(index) {
  if (state.treeSearchMatches.length === 0) return;
  state.treeSearchIndex = (index + state.treeSearchMatches.length) % state.treeSearchMatches.length;
  const selected = elements.treeSearchResults.children[state.treeSearchIndex];
  for (const [optionIndex, option] of [...elements.treeSearchResults.children].entries()) {
    option.setAttribute("aria-selected", String(optionIndex === state.treeSearchIndex));
  }
  elements.treeSearchInput.setAttribute("aria-activedescendant", selected.id);
  selected.scrollIntoView({ block: "nearest" });
}

async function openTreeSearch() {
  if (elements.treeSearchDialog.open) return;
  state.treeSearchReturnFocus = document.activeElement;
  state.treeSearchFuse = null;
  state.treeSearchEntries = [];
  state.treeSearchMatches = [];
  state.treeSearchIndex = -1;
  elements.treeSearchInput.value = "";
  elements.treeSearchResults.replaceChildren();
  elements.treeSearchStatus.textContent = "Loading tree…";
  elements.treeSearchDialog.showModal();
  elements.treeSearchInput.focus();

  const loadId = ++state.treeSearchLoadId;
  try {
    const response = await apiRequest("api/tree");
    const listing = await response.json();
    if (loadId !== state.treeSearchLoadId) return;
    state.treeSearchEntries = listing.entries;
    state.treeSearchFuse = new Fuse(state.treeSearchEntries, {
      keys: ["path"],
      threshold: 0.38,
      ignoreLocation: true,
      minMatchCharLength: 1,
    });
    if (elements.treeSearchDialog.open) updateTreeSearchResults();
  } catch (error) {
    if (loadId === state.treeSearchLoadId && elements.treeSearchDialog.open) {
      elements.treeSearchStatus.textContent = `Search failed: ${error.message}`;
    }
  }
}

function closeTreeSearch(restoreFocus = true) {
  if (!restoreFocus) state.treeSearchReturnFocus = null;
  if (elements.treeSearchDialog.open) elements.treeSearchDialog.close();
}

async function chooseTreeSearchResult(index) {
  const entry = state.treeSearchMatches[index];
  if (!entry) return;
  state.treeSearchReturnFocus = null;
  elements.treeSearchDialog.close();
  if (entry.kind === "file") {
    await openFile(entry.path);
    if (state.currentFile === entry.path) await revealFileInTree(entry.path);
  } else {
    await navigateToSearchDirectory(entry.path);
  }
}

async function revealFileInTree(path) {
  try {
    let ancestor = "";
    await fetchDirectory(ancestor);
    state.expandedDirectories.add(ancestor);
    for (const part of parentPath(path).split("/").filter(Boolean)) {
      ancestor = joinPath(ancestor, part);
      await fetchDirectory(ancestor);
      state.expandedDirectories.add(ancestor);
    }
    state.directory = parentPath(path);
    state.selectedPath = path;
    renderBrowser();
  } catch (error) {
    showError(elements.browserError, error);
  }
}

async function navigateToSearchDirectory(path) {
  clearError(elements.browserError);
  try {
    let ancestor = "";
    await fetchDirectory(ancestor);
    state.expandedDirectories.add(ancestor);
    for (const part of path.split("/")) {
      ancestor = joinPath(ancestor, part);
      await fetchDirectory(ancestor);
      state.expandedDirectories.add(ancestor);
    }
    state.directory = path;
    state.selectedPath = path;
    renderBrowser();
    focusTreeItem(path);
    closeMobileSidebar();
  } catch (error) {
    showError(elements.browserError, error);
  }
}

async function loadDirectory(path = state.directory) {
  clearError(elements.browserError);
  state.directory = path;
  state.expandedDirectories.add("");
  const parts = path ? path.split("/") : [];
  let ancestor = "";
  for (const part of parts) {
    ancestor = joinPath(ancestor, part);
    state.expandedDirectories.add(ancestor);
  }
  try {
    await fetchDirectory(path);
    renderBrowser();
  } catch (error) {
    showError(elements.browserError, error);
  }
}

function renderBrowser() {
  const tree = document.createDocumentFragment();
  const rootExpanded = state.expandedDirectories.has("");
  elements.rootMarker.replaceChildren(createChevronIcon(rootExpanded));
  elements.rootActions.setAttribute("aria-expanded", String(rootExpanded));
  elements.rootActions.setAttribute("aria-selected", String(state.selectedPath === ""));
  elements.rootActions.setAttribute(
    "aria-label",
    `Root directory /, ${rootExpanded ? "expanded" : "collapsed"}`,
  );
  elements.rootRow.classList.toggle("selected", state.selectedPath === "");
  if (rootExpanded) {
    renderDirectoryEntries(tree, state.entriesByDirectory.get("") || [], "", 1);
  }
  elements.treeEntries.replaceChildren(tree);
}

function renderDirectoryEntries(container, entries, directory, depth) {
  entries.forEach((entry, index) => {
    const path = joinPath(directory, entry.name);
    const row = document.createElement("div");
    row.className = "entry-row";
    row.setAttribute("role", "listitem");
    if (entry.kind === "other") row.classList.add("unsupported");
    if (path === state.selectedPath) row.classList.add("selected");
    const main = document.createElement("button");
    main.type = "button";
    main.className = "entry-main";
    main.dataset.kind = entry.kind;
    main.dataset.path = path;
    main.title = path;
    main.setAttribute(
      "aria-label",
      entry.kind === "directory"
        ? `${entry.name}, folder`
        : `${entry.name}${path === state.currentFile ? ", open" : ""}`,
    );
    main.setAttribute(
      "aria-selected",
      String(path === state.selectedPath),
    );
    if (entry.kind === "file" || entry.kind === "directory") {
      main.setAttribute("aria-haspopup", "menu");
    }

    main.style.paddingLeft = `${depth}rem`;

    const marker = document.createElement("span");
    marker.className = "tree-marker";
    marker.setAttribute("aria-hidden", "true");
    if (entry.kind === "directory") {
      const expanded = state.expandedDirectories.has(path);
      if (state.loadingDirectories.has(path)) marker.textContent = "..";
      else marker.append(createChevronIcon(expanded));
      marker.classList.add("folder-toggle");
      marker.title = `${expanded ? "Close" : "Open"} ${entry.name}`;
      marker.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        if (!state.loadingDirectories.has(path)) toggleDirectory(path);
      });
      marker.addEventListener("dblclick", (event) => {
        event.preventDefault();
        event.stopPropagation();
      });
      main.setAttribute("aria-expanded", String(expanded));
      main.setAttribute(
        "aria-label",
        `${entry.name}, folder, ${expanded ? "expanded" : "collapsed"}${path === state.directory ? ", current folder" : ""}`,
      );
      main.append(marker);
      main.append(createEntryIcon("folder"));
      const label = document.createElement("span");
      label.className = "entry-label";
      label.textContent = entry.name;
      main.append(label);
      main.addEventListener("click", () => selectNavigationItem(path, main));
      main.addEventListener("dblclick", () => toggleDirectory(path));
    } else if (entry.kind === "file") {
      marker.textContent = "    ";
      main.append(marker);
      main.append(createEntryIcon("text"));
      const label = document.createElement("span");
      label.className = "entry-label";
      label.textContent = entry.name;
      main.append(label);
      main.addEventListener("click", () => selectNavigationItem(path, main));
      main.addEventListener("dblclick", () => openFile(path));
    } else {
      marker.textContent = "    ";
      main.append(marker);
      const label = document.createElement("span");
      label.className = "entry-label";
      label.textContent = entry.name;
      main.append(label);
      main.disabled = true;
      main.title = "Unsupported filesystem entry";
    }
    if (entry.kind === "file" || entry.kind === "directory") {
      main.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        openEntryMenu({ ...entry, path }, event.clientX, event.clientY, main);
      });
    }
    row.append(main);

    container.append(row);

    if (entry.kind === "directory" && state.expandedDirectories.has(path)) {
      const childDepth = depth + 1;
      if (state.loadingDirectories.has(path)) {
        container.append(treeStatusRow(childDepth, "Loading…"));
      } else if (state.entriesByDirectory.has(path)) {
        const children = state.entriesByDirectory.get(path);
        if (children.length === 0) container.append(treeStatusRow(childDepth, "(empty)"));
        else renderDirectoryEntries(container, children, path, childDepth);
      }
    }
  });
}

function treeStatusRow(depth, label) {
  const row = document.createElement("div");
  row.className = "entry-row tree-status";
  row.setAttribute("role", "listitem");
  row.style.paddingLeft = `calc(${depth}rem + 2ch)`;
  row.textContent = label;
  return row;
}

function focusTreeItem(path) {
  if (path === "") {
    selectNavigationItem("", elements.rootActions);
    elements.rootActions.focus();
    return;
  }
  const button = [...elements.entryList.querySelectorAll(".entry-main")]
    .find((item) => item.dataset.path === path);
  if (button) {
    selectNavigationItem(path, button);
    button.focus();
  }
}

function toggleRoot() {
  if (state.expandedDirectories.has("")) state.expandedDirectories.delete("");
  else state.expandedDirectories.add("");
  state.selectedPath = "";
  state.directory = "";
  renderBrowser();
  focusTreeItem("");
}

function selectNavigationItem(path, target) {
  state.selectedPath = path;
  for (const selected of document.querySelectorAll(".entry-row.selected")) {
    selected.classList.remove("selected");
  }
  target.closest(".entry-row")?.classList.add("selected");
}

function openEntryMenu(entry, x, y, target) {
  selectNavigationItem(entry.path, target);
  state.contextEntry = entry;
  state.contextTarget = target;
  elements.deleteEntry.textContent = entry.kind === "directory" ? "Delete folder" : "Delete";
  const isFolder = entry.kind === "directory" || entry.kind === "root";
  elements.newFileInFolder.hidden = !isFolder;
  elements.newFolderInFolder.hidden = !isFolder;
  elements.renameEntry.hidden = entry.kind !== "file" && entry.kind !== "directory";
  elements.deleteEntry.hidden = entry.kind === "root";
  elements.entryMenu.hidden = false;

  const bounds = elements.entryMenu.getBoundingClientRect();
  const left = Math.max(4, Math.min(x, window.innerWidth - bounds.width - 4));
  const top = Math.max(4, Math.min(y, window.innerHeight - bounds.height - 4));
  elements.entryMenu.style.left = `${left}px`;
  elements.entryMenu.style.top = `${top}px`;
  (isFolder ? elements.newFileInFolder : elements.renameEntry).focus();
}

function openRootMenu(x, y) {
  openEntryMenu(
    { name: "/", kind: "root", path: "" },
    x,
    y,
    elements.rootActions,
  );
}

function closeEntryMenu(restoreFocus = false) {
  elements.entryMenu.hidden = true;
  if (restoreFocus) state.contextTarget?.focus();
}

function requestRenameEntry() {
  const entry = state.contextEntry;
  const returnFocus = state.contextTarget;
  closeEntryMenu();
  if (entry?.kind === "file" || entry?.kind === "directory") {
    openRenameDialog(entry, returnFocus);
  }
}

function createFileInContextFolder() {
  const entry = state.contextEntry;
  const returnFocus = state.contextTarget;
  closeEntryMenu();
  if (entry?.kind === "directory" || entry?.kind === "root") {
    openCreateDialog("file", entry.path, returnFocus);
  }
}

function createFolderInContextFolder() {
  const entry = state.contextEntry;
  const returnFocus = state.contextTarget;
  closeEntryMenu();
  if (entry?.kind === "directory" || entry?.kind === "root") {
    openCreateDialog("directory", entry.path, returnFocus);
  }
}

function requestDeleteEntry() {
  const entry = state.contextEntry;
  const returnFocus = state.contextTarget;
  closeEntryMenu();
  if (entry) openDeleteDialog(entry, returnFocus);
}

async function toggleDirectory(path) {
  state.selectedPath = path;
  state.directory = path;
  if (state.expandedDirectories.has(path)) {
    state.expandedDirectories.delete(path);
    renderBrowser();
    focusTreeItem(path);
    return;
  }

  state.expandedDirectories.add(path);
  if (!state.entriesByDirectory.has(path)) {
    state.loadingDirectories.add(path);
    renderBrowser();
    try {
      await fetchDirectory(path);
    } catch (error) {
      state.expandedDirectories.delete(path);
      showError(elements.browserError, error);
    } finally {
      state.loadingDirectories.delete(path);
    }
  }
  renderBrowser();
  focusTreeItem(path);
}

async function openFile(path) {
  state.selectedPath = path;
  if (path === state.currentFile) {
    focusTreeItem(path);
    closeMobileSidebar();
    elements.editor.focus();
    return;
  }
  if (!(await ensureSaved())) return;
  clearError(elements.browserError);
  try {
    const response = await apiRequest("api/file", {}, path);
    const bytes = await response.arrayBuffer();
    const text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
    state.currentFile = path;
    state.selectedPath = path;
    state.directory = parentPath(path);
    state.rawText = text;
    state.normalizedText = normalizeNewlines(text);
    state.newline = dominantNewline(text);
    state.editVersion = 0;
    state.dirty = false;
    state.lastSaveOk = true;
    elements.editor.value = state.normalizedText;
    elements.editor.disabled = false;
    elements.filename.textContent = path;
    setSaveStatus("Saved");
    renderBrowser();
    closeMobileSidebar();
    elements.editor.focus();
  } catch (error) {
    showError(elements.browserError, error);
  }
}

function normalizeNewlines(text) {
  return text.replace(/\r\n|\r/g, "\n");
}

function dominantNewline(text) {
  const crlf = (text.match(/\r\n/g) || []).length;
  const withoutCrlf = text.replace(/\r\n/g, "");
  const cr = (withoutCrlf.match(/\r/g) || []).length;
  const lf = (withoutCrlf.match(/\n/g) || []).length;
  if (crlf >= cr && crlf >= lf && crlf > 0) return "\r\n";
  if (cr > lf && cr > 0) return "\r";
  return "\n";
}

function normalizedOffsetToRaw(raw, offset) {
  let rawOffset = 0;
  let normalizedOffset = 0;
  while (rawOffset < raw.length && normalizedOffset < offset) {
    if (raw[rawOffset] === "\r" && raw[rawOffset + 1] === "\n") {
      rawOffset += 2;
    } else {
      rawOffset += 1;
    }
    normalizedOffset += 1;
  }
  return rawOffset;
}

function applyEditorChange(nextNormalized) {
  const previous = state.normalizedText;
  let prefix = 0;
  while (prefix < previous.length && prefix < nextNormalized.length && previous[prefix] === nextNormalized[prefix]) {
    prefix += 1;
  }
  let suffix = 0;
  while (
    suffix < previous.length - prefix &&
    suffix < nextNormalized.length - prefix &&
    previous[previous.length - 1 - suffix] === nextNormalized[nextNormalized.length - 1 - suffix]
  ) {
    suffix += 1;
  }
  const oldEnd = previous.length - suffix;
  const newEnd = nextNormalized.length - suffix;
  const rawStart = normalizedOffsetToRaw(state.rawText, prefix);
  const rawEnd = normalizedOffsetToRaw(state.rawText, oldEnd);
  const insertion = nextNormalized.slice(prefix, newEnd).replace(/\n/g, state.newline);
  state.rawText = state.rawText.slice(0, rawStart) + insertion + state.rawText.slice(rawEnd);
  state.normalizedText = nextNormalized;
}

function scheduleSave() {
  window.clearTimeout(state.saveTimer);
  state.saveTimer = window.setTimeout(() => saveNow(), 600);
}

async function saveNow() {
  window.clearTimeout(state.saveTimer);
  if (!state.currentFile) return true;
  if (state.savePromise) {
    await state.savePromise;
    if (state.dirty && state.lastSaveOk) return saveNow();
    return state.lastSaveOk;
  }
  if (!state.dirty) return true;

  const version = state.editVersion;
  const payload = new TextEncoder().encode(state.rawText);
  setSaveStatus("Saving…");
  state.savePromise = apiRequest(
    "api/file",
    { method: "PUT", headers: { "Content-Type": "text/plain; charset=utf-8" }, body: payload },
    state.currentFile,
  );

  try {
    await state.savePromise;
    state.lastSaveOk = true;
    if (state.editVersion === version) {
      state.dirty = false;
      setSaveStatus("Saved");
    } else {
      setSaveStatus("Unsaved");
    }
  } catch (error) {
    state.lastSaveOk = false;
    state.dirty = true;
    setSaveStatus("Save failed", true);
    elements.saveStatus.title = error.message;
  } finally {
    state.savePromise = null;
  }

  if (state.dirty && state.lastSaveOk) return saveNow();
  return state.lastSaveOk;
}

async function ensureSaved() {
  if (!state.dirty && !state.savePromise) return true;
  const saved = await saveNow();
  if (!saved) elements.editor.focus();
  return saved;
}

function openCreateDialog(kind, directory = state.directory, returnFocus = null) {
  closeEntryMenu();
  const isFile = kind === "file";
  openActionDialog({
    kind: isFile ? "create-file" : "create-directory",
    title: isFile ? "New file" : "New folder",
    directory,
    label: isFile ? "Filename" : "Folder name",
    placeholder: isFile ? "untitled.txt" : "new-folder",
    submit: "Create",
    returnFocus,
  });
}

function openRenameDialog(entry, returnFocus = null) {
  openActionDialog({
    kind: "rename",
    entry,
    title: entry.kind === "directory" ? "Rename folder" : "Rename file",
    directory: parentPath(entry.path),
    label: "New filename",
    value: basename(entry.path),
    submit: "Rename",
    selectValue: true,
    returnFocus,
  });
}

function openDeleteDialog(entry, returnFocus = null) {
  const isDirectory = entry.kind === "directory";
  openActionDialog({
    kind: "delete",
    entry,
    title: isDirectory ? "Delete folder and contents?" : "Delete file?",
    directory: parentPath(entry.path),
    message: isDirectory
      ? `Delete “${entry.name}” and everything inside it permanently?`
      : `Delete “${entry.name}” permanently?`,
    submit: "Delete",
    danger: true,
    returnFocus,
  });
}

function openActionDialog(options) {
  closeEntryMenu();
  state.actionKind = options.kind;
  state.actionDirectory = options.directory;
  state.actionEntry = options.entry || null;
  state.actionReturnFocus = options.returnFocus || null;
  const needsName = Boolean(options.label);

  elements.actionTitle.textContent = options.title;
  elements.actionLocation.textContent = `in ${options.directory ? `/${options.directory}` : "/"}`;
  elements.actionLabel.textContent = options.label || "";
  elements.actionLabel.hidden = !needsName;
  elements.actionName.hidden = !needsName;
  elements.actionName.required = needsName;
  elements.actionName.value = options.value || "";
  elements.actionName.placeholder = options.placeholder || "";
  elements.actionMessage.textContent = options.message || "";
  elements.actionMessage.hidden = !options.message;
  elements.actionSubmit.textContent = options.submit;
  elements.actionSubmit.classList.toggle("danger", Boolean(options.danger));
  clearError(elements.actionError);
  elements.actionDialog.showModal();
  if (needsName) {
    elements.actionName.focus();
    if (options.selectValue) elements.actionName.select();
  } else {
    elements.actionSubmit.focus();
  }
}

async function submitAction(event) {
  event.preventDefault();
  clearError(elements.actionError);
  const { actionKind, actionDirectory, actionEntry } = state;
  const name = elements.actionName.value;
  elements.actionSubmit.disabled = true;

  try {
    if (actionKind === "create-file" || actionKind === "create-directory") {
      const isFile = actionKind === "create-file";
      const response = await apiRequest(isFile ? "api/file" : "api/directory", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ directory: actionDirectory, name }),
      });
      const created = await response.json();
      state.actionReturnFocus = null;
      elements.actionDialog.close();
      await loadDirectory(actionDirectory);
      if (isFile) await openFile(created.path);
      else focusTreeItem(created.path);
      return;
    }

    if (actionKind === "rename") {
      if (name === basename(actionEntry.path)) {
        elements.actionDialog.close();
        return;
      }
      const oldPath = actionEntry.path;
      const isDirectory = actionEntry.kind === "directory";
      const openFileAffected = Boolean(state.currentFile && (
        state.currentFile === oldPath ||
        (isDirectory && state.currentFile.startsWith(`${oldPath}/`))
      ));
      if (openFileAffected && !(await ensureSaved())) {
        throw new Error("The open file could not be saved. Rename was not applied.");
      }
      const response = await apiRequest(isDirectory ? "api/directory" : "api/file", {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path: oldPath, name }),
      });
      const renamed = await response.json();
      const nextPath = renamed.path;
      const refreshPath = state.directory === oldPath || state.directory.startsWith(`${oldPath}/`)
        ? rebasePath(state.directory, oldPath, nextPath)
        : parentPath(oldPath);
      if (isDirectory) {
        const cachedEntries = [...state.entriesByDirectory.entries()];
        state.entriesByDirectory.clear();
        for (const [path, entries] of cachedEntries) {
          state.entriesByDirectory.set(rebasePath(path, oldPath, nextPath), entries);
        }
        state.expandedDirectories = new Set(
          [...state.expandedDirectories].map((path) => rebasePath(path, oldPath, nextPath)),
        );
      }
      state.currentFile = state.currentFile
        ? rebasePath(state.currentFile, oldPath, nextPath)
        : null;
      state.selectedPath = state.selectedPath === null
        ? null
        : rebasePath(state.selectedPath, oldPath, nextPath);
      if (state.currentFile) elements.filename.textContent = state.currentFile;
      state.directory = rebasePath(state.directory, oldPath, nextPath);
      state.actionReturnFocus = null;
      elements.actionDialog.close();
      await loadDirectory(refreshPath);
      focusTreeItem(nextPath);
      return;
    }

    if (actionKind === "delete") {
      const isDirectory = actionEntry.kind === "directory";
      const openFileAffected = Boolean(state.currentFile && (
        state.currentFile === actionEntry.path ||
        (isDirectory && state.currentFile.startsWith(`${actionEntry.path}/`))
      ));
      if (openFileAffected && !(await ensureSaved())) {
        throw new Error("The open file could not be saved. Delete was cancelled.");
      }
      await apiRequest(
        isDirectory ? "api/directory" : "api/file",
        { method: "DELETE" },
        actionEntry.path,
      );

      if (openFileAffected) {
        window.clearTimeout(state.saveTimer);
        state.saveTimer = null;
        state.currentFile = null;
        state.rawText = "";
        state.normalizedText = "";
        state.dirty = false;
        state.lastSaveOk = true;
        elements.editor.value = "";
        elements.editor.disabled = true;
        elements.filename.textContent = "No file open";
        setSaveStatus("");
      }

      const parent = parentPath(actionEntry.path);
      state.selectedPath = parent;
      if (isDirectory) {
        state.entriesByDirectory.delete(actionEntry.path);
        for (const cachedPath of state.entriesByDirectory.keys()) {
          if (cachedPath.startsWith(`${actionEntry.path}/`)) state.entriesByDirectory.delete(cachedPath);
        }
        for (const expandedPath of state.expandedDirectories) {
          if (expandedPath === actionEntry.path || expandedPath.startsWith(`${actionEntry.path}/`)) {
            state.expandedDirectories.delete(expandedPath);
          }
        }
        if (state.directory === actionEntry.path || state.directory.startsWith(`${actionEntry.path}/`)) {
          state.directory = parent;
        }
      }
      state.actionReturnFocus = null;
      elements.actionDialog.close();
      await loadDirectory(parent);
      focusTreeItem(parent);
    }
  } catch (error) {
    showError(elements.actionError, error);
  } finally {
    elements.actionSubmit.disabled = false;
  }
}

elements.editor.addEventListener("input", () => {
  applyEditorChange(elements.editor.value);
  state.editVersion += 1;
  state.dirty = true;
  state.lastSaveOk = true;
  setSaveStatus("Unsaved");
  scheduleSave();
});

elements.findTree.addEventListener("click", openTreeSearch);
elements.closeTreeSearch.addEventListener("click", () => closeTreeSearch());
elements.treeSearchInput.addEventListener("input", updateTreeSearchResults);
elements.treeSearchDialog.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    closeTreeSearch();
  } else if (
    (event.key === "ArrowDown" || event.key === "ArrowUp") &&
    state.treeSearchMatches.length > 0
  ) {
    event.preventDefault();
    const direction = event.key === "ArrowDown" ? 1 : -1;
    setTreeSearchSelection(state.treeSearchIndex + direction);
  } else if (event.key === "Enter" && event.target !== elements.closeTreeSearch) {
    event.preventDefault();
    chooseTreeSearchResult(state.treeSearchIndex);
  }
});
elements.treeSearchDialog.addEventListener("close", () => {
  const target = state.treeSearchReturnFocus;
  state.treeSearchReturnFocus = null;
  if (target?.isConnected) target.focus();
});

elements.actionForm.addEventListener("submit", submitAction);
elements.newFileInFolder.addEventListener("click", createFileInContextFolder);
elements.newFolderInFolder.addEventListener("click", createFolderInContextFolder);
elements.renameEntry.addEventListener("click", requestRenameEntry);
elements.deleteEntry.addEventListener("click", requestDeleteEntry);
elements.openSidebar.addEventListener("click", openMobileSidebar);
elements.closeSidebar.addEventListener("click", closeMobileSidebar);
elements.rootActions.addEventListener("click", () => {
  selectNavigationItem("", elements.rootActions);
});
elements.rootActions.addEventListener("dblclick", toggleRoot);
elements.rootActions.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  openRootMenu(event.clientX, event.clientY);
});
elements.rootMarker.addEventListener("click", (event) => {
  event.preventDefault();
  event.stopPropagation();
  toggleRoot();
});
elements.rootMarker.addEventListener("dblclick", (event) => {
  event.preventDefault();
  event.stopPropagation();
});

elements.entryList.addEventListener("keydown", async (event) => {
  const current = event.target.closest(".entry-main");
  if (!current) return;
  if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
    event.preventDefault();
    const bounds = current.getBoundingClientRect();
    const entry = {
      name: current.dataset.kind === "root" ? "/" : basename(current.dataset.path),
      kind: current.dataset.kind,
      path: current.dataset.path,
    };
    if (entry.kind === "root") openRootMenu(bounds.left, bounds.bottom);
    else openEntryMenu(entry, bounds.left, bounds.bottom, current);
    return;
  }
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    selectNavigationItem(current.dataset.path, current);
    if (current.dataset.kind === "root") {
      toggleRoot();
    } else if (current.dataset.kind === "directory") {
      await toggleDirectory(current.dataset.path);
    } else if (current.dataset.kind === "file") {
      await openFile(current.dataset.path);
    }
    return;
  }
  const buttons = [...elements.entryList.querySelectorAll(".entry-main:not(:disabled)")];
  const index = buttons.indexOf(current);
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    const offset = event.key === "ArrowDown" ? 1 : -1;
    if (event.key === "ArrowUp" && index === 0) {
      focusTreeItem("");
      return;
    }
    const next = buttons[Math.max(0, Math.min(buttons.length - 1, index + offset))];
    if (next) focusTreeItem(next.dataset.path);
  } else if (event.key === "ArrowRight" && (current.dataset.kind === "directory" || current.dataset.kind === "root")) {
    event.preventDefault();
    const path = current.dataset.path;
    if (!state.expandedDirectories.has(path)) {
      if (current.dataset.kind === "root") toggleRoot();
      else await toggleDirectory(path);
    } else {
      const next = buttons[index + 1];
      if (next) focusTreeItem(next.dataset.path);
    }
  } else if (event.key === "ArrowLeft") {
    event.preventDefault();
    const path = current.dataset.path;
    if (current.dataset.kind === "root" && state.expandedDirectories.has(path)) {
      toggleRoot();
    } else if (current.dataset.kind === "directory" && state.expandedDirectories.has(path)) {
      state.expandedDirectories.delete(path);
      renderBrowser();
      focusTreeItem(path);
    } else {
      const parent = parentPath(path);
      focusTreeItem(parent);
    }
  }
});

for (const button of document.querySelectorAll("[data-close-dialog]")) {
  button.addEventListener("click", () => button.closest("dialog").close());
}

elements.actionDialog.addEventListener("close", () => {
  const target = state.actionReturnFocus;
  state.actionReturnFocus = null;
  if (target?.isConnected) target.focus();
});

document.addEventListener("keydown", (event) => {
  if (!elements.entryMenu.hidden) {
    if (event.key === "Escape") {
      event.preventDefault();
      closeEntryMenu(true);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const items = [...elements.entryMenu.querySelectorAll("button:not([hidden])")];
      const currentIndex = items.indexOf(document.activeElement);
      const step = event.key === "ArrowDown" ? 1 : -1;
      const nextIndex = currentIndex < 0
        ? 0
        : (currentIndex + step + items.length) % items.length;
      items[nextIndex]?.focus();
      event.preventDefault();
      return;
    }
  }
  const command = event.ctrlKey || event.metaKey;
  if (command && event.key.toLowerCase() === "s") {
    event.preventDefault();
    saveNow();
  } else if (command && event.key.toLowerCase() === "n") {
    event.preventDefault();
    if (elements.treeSearchDialog.open) closeTreeSearch(false);
    openCreateDialog("file");
  } else if (command && event.key.toLowerCase() === "o") {
    event.preventDefault();
    if (elements.treeSearchDialog.open) closeTreeSearch(false);
    if (window.matchMedia("(max-width: 700px)").matches) openMobileSidebar();
    else {
      const first = elements.entryList.querySelector("button:not(:disabled)");
      if (first) focusTreeItem(first.dataset.path);
      else focusTreeItem("");
    }
  } else if (event.key === "Escape" && elements.sidebar.classList.contains("open")) {
    closeMobileSidebar();
    elements.openSidebar.focus();
  }
});

document.addEventListener("pointerdown", (event) => {
  if (!elements.entryMenu.hidden && !elements.entryMenu.contains(event.target)) {
    closeEntryMenu();
  }
});

elements.entryMenu.addEventListener("focusout", (event) => {
  if (!elements.entryMenu.contains(event.relatedTarget)) closeEntryMenu();
});

window.addEventListener("resize", () => {
  closeEntryMenu();
  setSidebarWidth(state.sidebarWidth);
});
window.addEventListener("scroll", () => closeEntryMenu(), true);

window.addEventListener("beforeunload", (event) => {
  if (state.dirty || state.savePromise) {
    event.preventDefault();
    event.returnValue = "";
  }
});

elements.rootActions.insertBefore(
  createEntryIcon("folder"),
  elements.rootActions.querySelector(".entry-label"),
);

loadDirectory();
