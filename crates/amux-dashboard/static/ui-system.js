// AMUX Basecoat bridge
//
// The dashboard predates Basecoat and renders a substantial amount of HTML at
// runtime. Rewriting every renderer would create a second, incomplete UI. This
// bridge makes the existing semantic elements participate in one Basecoat
// vocabulary, then keeps ARIA state in sync as app.js changes the DOM.
(function installAmuxBasecoatUI() {
  'use strict';

  const TOP_LEVEL_VIEWS = [
    'session-view', 'groups-view', 'board-view', 'calendar-view',
    'scheduler-view', 'files-view', 'mdai-view', 'proxies-view', 'email-view',
    'connectors-view', 'logs-view', 'map-view', 'cost-view', 'metrics-view',
    'torrents-view', 'sql-view', 'messages-view', 'terminal-view',
    'browser-view', 'graph-view', 'journal-view', 'habits-view', 'trends-view',
    'skills-view', 'grid-view',
  ];

  const TABLIST_SELECTORS = [
    '.tab-bar', '.peek-tab-list', '.settings-tabs', '.logs-subtabs',
    '.board-detail-tabs', '.file-mode-tabs', '.dt-tabs', '.jrnl-view-tabs',
    '.report-period-tabs', '.dict-subtabs', '.xlsx-tabs', '.mdai-btabs',
    '.db-modes', '.metrics-modebar', '.sm-scope-tabs', '.msg-mode-tabs',
    '.notes-mode-tabs', '.bw-inspector-tabs',
  ];

  const TOGGLE_GROUPS = [
    ['bo-human', 'bo-agent'],
    ['bv-list', 'bv-session', 'bv-status'],
    ['piv-list', 'piv-kanban'],
    ['graph-switch-default', 'graph-switch-fleet'],
  ];

  const MENU_SELECTORS = [
    '.card-menu', '.header-add-menu', '.active-dropdown', '.sort-menu',
    '.peek-more-dropdown', '.fe-tb-overflow-menu', '.msg-menu',
    '.ws-note-menu', '.ws-preset-menu', '.tab-customizer-menu',
  ];

  const MENU_ITEM_SELECTORS = [
    '.card-menu-item', '.peek-more-item', '.fe-tb-oitem',
    '.active-dropdown-item', '.msg-menu > button', '.tab-customizer-item',
  ];

  const DIALOG_SELECTORS = [
    '#apikey-setup-modal', '#proxy-form-overlay', '#map-pin-modal',
    '#map-tag-modal', '#video-overlay', '#teleprompter-overlay',
    '#sched-overlay', '#board-edit-overlay', '#board-detail-overlay',
    '#orch-overlay', '#create-overlay', '#connect-overlay',
    '#iterm2-connect-overlay', '#subagents-overlay', '#peek-overlay',
    '#edit-overlay', '#queue-overlay', '#about-overlay', '#cmd-history-modal',
    '#filters-modal', '#saved-messages-modal', '#channel-drawer',
    '#scope-edit-backdrop', '#modal-backdrop', '#bulk-actions-overlay',
    '#skill-edit-modal', '#file-overlay', '#mdai-overlay',
    '#wt-overlay', '.tts-overlay', '.chip-picker-overlay', '[data-ical-modal]',
  ];

  const CARD_SELECTORS = [
    '.board-card', '.archived-card', '.conn-card', '.cost-card',
    '.metrics-card', '.metrics-speedtest-card', '.habit-card', '.skill-card',
    '.focus-card', '.report-card', '.tr-card', '.lib-card', '.jrnl-entry-card',
    '.reclaim-catcard', '.overlay-card',
  ];

  const BADGE_SELECTORS = [
    '.status-badge', '.task-id-chip', '.board-card-tag', '.board-card-key',
    '.conn-badge', '.mdai-badge', '.branch-badge', '.cached-badge',
    '.draft-badge', '.upq-badge', '.notif-badge', '.peek-tab-count',
    '.msg-kind-chip', '.msg-card-chip', '.msg-id-badge', '.sched-id-badge',
    '.archived-card-tag', '.archived-card-chip',
  ];

  const EMPTY_SELECTORS = [
    '.empty', '.board-empty', '.board-session-empty', '.board-archived-empty',
    '.active-dropdown-empty', '.connect-empty', '.conn-empty', '.gmail-empty',
    '.habit-empty', '.queue-empty', '.sysjob-empty', '.git-diff-empty',
    '.commits-detail-empty', '.jrnl-editor-empty', '.reclaim-empty',
    '.xlsx-empty', '.il-empty', '.channel-empty',
  ];

  const TOOLBAR_SELECTORS = [
    '.header-row', '.board-toolbar', '.logs-toolbar', '.fe-toolbar',
    '.term-toolbar', '.grid-toolbar', '.jrnl-toolbar', '.map-toolbar',
    '.peek-board-toolbar', '.db-toolbar', '.csv-toolbar', '.dt-toolbar',
    '.file-view-tabs',
  ];

  const TRIGGER_PAIRS = [
    ['notif-btn', 'notif-panel'], ['active-btn', 'active-dropdown'],
    ['add-btn', 'add-menu'], ['settings-btn', 'settings-menu'],
    ['files-overflow-btn', 'files-overflow-menu'],
    ['peek-more-btn', 'peek-more-dropdown'],
    ['peek-tab-customize', 'peek-tab-customizer-menu'],
  ];

  function selectIncludingRoot(root, selector) {
    const out = [];
    if (root && root.nodeType === 1 && root.matches(selector)) out.push(root);
    if (root && root.querySelectorAll) out.push(...root.querySelectorAll(selector));
    return out;
  }

  function setAttr(el, name, value) {
    if (!el || el.getAttribute(name) === value) return;
    el.setAttribute(name, value);
  }

  function addClass(el, name) {
    if (el && !el.classList.contains(name)) el.classList.add(name);
  }

  function labelDialog(el) {
    if (el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby')) return;
    const title = el.querySelector('h1,h2,h3,[data-title],.modal-title,.map-modal-title');
    if (title) {
      if (!title.id) title.id = `${el.id || 'amux-dialog'}-title`;
      setAttr(el, 'aria-labelledby', title.id);
    } else {
      setAttr(el, 'aria-label', 'AMUX dialog');
    }
  }

  function enhanceViews(root) {
    for (const id of TOP_LEVEL_VIEWS) {
      const el = root.getElementById ? root.getElementById(id) : document.getElementById(id);
      if (!el) continue;
      addClass(el, 'amux-view');
      setAttr(el, 'data-ui-page', id.replace(/-view$/, ''));
      setAttr(el, 'role', 'tabpanel');
      setAttr(el, 'aria-hidden', isShown(el) ? 'false' : 'true');
    }
  }

  function enhanceButtons(root) {
    for (const button of selectIncludingRoot(root, 'button')) {
      addClass(button, 'btn');
      setAttr(button, 'data-ui-control', 'button');

      const cls = button.classList;
      const label = (button.textContent || '').trim().toLowerCase();
      let variant = button.getAttribute('data-variant');
      if (!variant) {
        if (cls.contains('danger') || cls.contains('term-danger') ||
            cls.contains('map-btn-danger') || /^(delete|remove|disconnect)$/.test(label)) {
          variant = 'destructive';
        } else if (cls.contains('primary') || cls.contains('header-add-btn') ||
                   cls.contains('board-new-btn') || cls.contains('map-drop-btn')) {
          variant = 'primary';
        } else if (button.closest('[data-ui-tablist],[data-ui-menu]') ||
                   cls.contains('tile-btn') || cls.contains('notes-toggle-btn') ||
                   cls.contains('notes-new-btn')) {
          variant = 'ghost';
        } else {
          variant = 'outline';
        }
        setAttr(button, 'data-variant', variant);
      }

      if (!button.hasAttribute('data-size')) {
        const text = (button.textContent || '').trim();
        const title = button.getAttribute('title') || button.getAttribute('aria-label');
        const iconOnly = !!title && (text.length <= 2 || button.children.length === 1 && !text);
        if (iconOnly) setAttr(button, 'data-size', 'icon-sm');
        else if (cls.contains('lf-btn') || cls.contains('bv-btn') ||
                 cls.contains('file-view-tab') || cls.contains('peek-tab')) {
          setAttr(button, 'data-size', 'sm');
        }
      }

      if (!button.hasAttribute('aria-label') && !button.textContent.trim()) {
        const title = button.getAttribute('title');
        if (title) setAttr(button, 'aria-label', title);
      }
    }
  }

  function enhanceFields(root) {
    const textual = "input:not([type]),input[type='text'],input[type='email'],input[type='password'],input[type='number'],input[type='tel'],input[type='url'],input[type='search'],input[type='date'],input[type='datetime-local'],input[type='month'],input[type='week'],input[type='time']";
    for (const input of selectIncludingRoot(root, textual)) {
      addClass(input, 'input');
      setAttr(input, 'data-ui-control', 'input');
    }
    for (const select of selectIncludingRoot(root, 'select')) {
      addClass(select, 'select');
      setAttr(select, 'data-ui-control', 'select');
    }
    for (const textarea of selectIncludingRoot(root, 'textarea')) {
      addClass(textarea, 'textarea');
      setAttr(textarea, 'data-ui-control', 'textarea');
    }
    for (const checkbox of selectIncludingRoot(root, "input[type='checkbox'],input[type='radio']")) {
      addClass(checkbox, checkbox.type === 'radio' ? 'radio' : 'checkbox');
      setAttr(checkbox, 'data-ui-control', checkbox.type);
    }
    for (const range of selectIncludingRoot(root, "input[type='range']")) {
      addClass(range, 'range');
      setAttr(range, 'data-ui-control', 'range');
    }
    for (const group of selectIncludingRoot(root, '.field-group,.dict-modal-row,.conn-add-grid > label')) {
      addClass(group, 'field');
      setAttr(group, 'data-ui-component', 'field');
    }
  }

  function ensureTabId(tab, fallback) {
    if (!tab.id && fallback) tab.id = fallback;
    return tab.id;
  }

  function panelIdsForTab(tab, index) {
    const list = tab.parentElement;
    if (tab.dataset.stab) {
      ensureTabId(tab, `settings-tab-${tab.dataset.stab}`);
      return [`stab-${tab.dataset.stab}`];
    }
    if (tab.dataset.view && list && list.classList.contains('jrnl-view-tabs')) {
      ensureTabId(tab, `journal-tab-${tab.dataset.view}`);
      return [`jrnl-${tab.dataset.view}-pane`];
    }
    if (tab.classList.contains('dt-tab')) {
      const name = tab.textContent.trim().toLowerCase();
      ensureTabId(tab, `devtools-tab-${name}`);
      return [`dt-panel-${name}`];
    }
    if (tab.dataset.itab && list && list.classList.contains('bw-inspector-tabs')) {
      ensureTabId(tab, `browser-inspector-tab-${tab.dataset.itab}`);
      return ['bw-inspect-list'];
    }
    if (tab.classList.contains('xlsx-tab')) {
      ensureTabId(tab, `xlsx-tab-${index}`);
      return [`xlsx-sheet-${index}`];
    }
    if (tab.classList.contains('mdai-btab')) {
      const panel = list && list.nextElementSibling;
      if (panel && panel.classList.contains('mdai-btab-content')) {
        if (!panel.id) panel.id = 'mdai-bottom-panel';
        ensureTabId(tab, `mdai-tab-${index}`);
        return [panel.id];
      }
    }
    const staticPanels = {
      'lst-activity': ['logs-activity'],
      'lst-raw': ['logs-raw'],
      'lst-stats': ['logs-stats'],
      'lst-health': ['logs-health'],
      'db-mode-data': ['db-pane-data'],
      'db-mode-structure': ['db-pane-structure'],
      'db-mode-query': ['db-pane-query'],
      'metricsmode-system': ['metrics-content'],
      'metricsmode-disk': ['metrics-content'],
      'dict-subtab-history': ['dict-body'],
      'dict-subtab-dict': ['dict-body'],
      'dict-subtab-settings': ['dict-body'],
      'sm-scope-session': ['sm-list'],
      'sm-scope-all': ['sm-list'],
      'file-tab-preview': ['file-body'],
      'file-tab-edit': ['file-edit-wrap'],
      'file-tab-raw': ['file-body'],
      'pm-tab-edit': ['peek-memory-input'],
      'pm-tab-preview': ['peek-memory-preview'],
      'pm-tab-global': ['peek-global-input'],
      'pm-tab-inherited': ['peek-memory-inherited'],
      'bd-tab-preview': ['bd-meta', 'bd-preview'],
      'bd-tab-history': ['bd-log'],
      'bd-tab-edit': ['bd-edit-fields', 'bd-desc'],
      'msgmode-messages': ['msgs-list'],
      'msgmode-trends': ['trends-view'],
      'sched-cmd-tab-edit': ['sched-command-editor-wrap'],
      'sched-cmd-tab-preview': ['sched-command-preview'],
    };
    if (tab.id && staticPanels[tab.id]) return staticPanels[tab.id];
    if (!tab.id) return [];
    if (tab.id.indexOf('tab-') === 0 && tab.id.indexOf('peek-tab-') !== 0) {
      const name = tab.id.slice(4);
      if (name === 'sessions') return ['session-view'];
      if (name === 'grid') return ['grid-view'];
      return [`${name}-view`];
    }
    if (tab.id.indexOf('peek-tab-') === 0) {
      const name = tab.id.slice('peek-tab-'.length);
      const candidate = `peek-${name}-panel`;
      return document.getElementById(candidate) ? [candidate] : [];
    }
    return (tab.getAttribute('aria-controls') || '').split(/\s+/).filter(Boolean);
  }

  function syncTablist(list) {
    setAttr(list, 'role', 'tablist');
    setAttr(list, 'aria-orientation', 'horizontal');
    setAttr(list, 'data-ui-tablist', '');
    setAttr(list, 'data-ui-component', 'tabs');
    const listVisible = list.getClientRects().length > 0 && isShown(list);

    const tabs = Array.from(list.querySelectorAll(':scope > button'));
    const panels = new Map();
    for (const [index, tab] of tabs.entries()) {
      if (!(tab instanceof HTMLElement)) continue;
      setAttr(tab, 'role', 'tab');
      const active = tab.classList.contains('active') || tab.classList.contains('selected');
      setAttr(tab, 'aria-selected', active ? 'true' : 'false');
      setAttr(tab, 'tabindex', active ? '0' : '-1');
      const panelIds = panelIdsForTab(tab, index).filter(id => document.getElementById(id));
      if (panelIds.length) setAttr(tab, 'aria-controls', panelIds.join(' '));
      for (const panelId of panelIds) {
        if (!panels.has(panelId)) panels.set(panelId, []);
        panels.get(panelId).push({ tab, active });
      }
    }
    for (const [panelId, controllers] of panels) {
      const panel = document.getElementById(panelId);
      const selected = controllers.find(controller => controller.active && listVisible);
      setAttr(panel, 'role', 'tabpanel');
      setAttr(panel, 'aria-hidden', selected ? 'false' : 'true');
      const labelledBy = (selected || controllers[0]).tab.id;
      if (labelledBy) setAttr(panel, 'aria-labelledby', labelledBy);
    }
  }

  function enhanceTabs(root) {
    for (const selector of TABLIST_SELECTORS) {
      for (const list of selectIncludingRoot(root, selector)) syncTablist(list);
    }
  }

  function enhanceToggleGroups() {
    for (const ids of TOGGLE_GROUPS) {
      const buttons = ids.map(id => document.getElementById(id)).filter(Boolean);
      if (!buttons.length) continue;
      const group = buttons[0].parentElement;
      if (group) {
        setAttr(group, 'role', 'group');
        setAttr(group, 'data-ui-component', 'segmented-control');
      }
      for (const button of buttons) {
        setAttr(button, 'aria-pressed', button.classList.contains('active') ? 'true' : 'false');
      }
    }
    for (const group of document.querySelectorAll('.sched-mode-seg')) {
      setAttr(group, 'role', 'group');
      setAttr(group, 'aria-label', 'Schedule type');
      setAttr(group, 'data-ui-component', 'segmented-control');
      for (const button of group.querySelectorAll('.sched-mode-btn')) {
        setAttr(button, 'aria-pressed', button.classList.contains('active') ? 'true' : 'false');
      }
    }
  }

  function enhanceMenus(root) {
    for (const selector of MENU_SELECTORS) {
      for (const menu of selectIncludingRoot(root, selector)) {
        setAttr(menu, 'role', 'menu');
        setAttr(menu, 'data-ui-menu', '');
        setAttr(menu, 'data-ui-component', 'dropdown-menu');
      }
    }
    for (const selector of MENU_ITEM_SELECTORS) {
      for (const item of selectIncludingRoot(root, selector)) {
        if (item.closest('[data-ui-menu]')) {
          setAttr(item, 'role', item.querySelector("input[type='checkbox']") ? 'menuitemcheckbox' : 'menuitem');
          if (!(item instanceof HTMLButtonElement) && !item.hasAttribute('tabindex')) setAttr(item, 'tabindex', '0');
        }
      }
    }
  }

  function enhanceSurfaces(root) {
    for (const component of selectIncludingRoot(root, '[data-component]')) {
      setAttr(component, 'data-ui-component', component.getAttribute('data-component'));
    }
    for (const selector of CARD_SELECTORS) {
      for (const card of selectIncludingRoot(root, selector)) addClass(card, 'card');
    }
    for (const selector of BADGE_SELECTORS) {
      for (const badge of selectIncludingRoot(root, selector)) addClass(badge, 'badge');
    }
    for (const selector of EMPTY_SELECTORS) {
      for (const empty of selectIncludingRoot(root, selector)) {
        addClass(empty, 'empty');
        setAttr(empty, 'data-ui-empty', '');
      }
    }
    for (const selector of TOOLBAR_SELECTORS) {
      for (const toolbar of selectIncludingRoot(root, selector)) setAttr(toolbar, 'data-ui-toolbar', '');
    }
    for (const table of selectIncludingRoot(root, 'table')) addClass(table, 'table');
    for (const loading of selectIncludingRoot(root, '.peek-loading,.spinner,.peek-spin,.peek-spin-lg')) {
      setAttr(loading, 'data-ui-loading', '');
      setAttr(loading, 'aria-live', 'polite');
    }
  }

  function enhanceDialogs(root) {
    for (const selector of DIALOG_SELECTORS) {
      for (const dialog of selectIncludingRoot(root, selector)) {
        setAttr(dialog, 'role', 'dialog');
        setAttr(dialog, 'aria-modal', 'true');
        setAttr(dialog, 'data-ui-dialog', '');
        setAttr(dialog, 'data-ui-component', 'dialog');
        setAttr(dialog, 'aria-hidden', isShown(dialog) ? 'false' : 'true');
        labelDialog(dialog);
      }
    }
  }

  function isShown(el) {
    if (!el) return false;
    if (el.classList.contains('open') || el.classList.contains('active')) return true;
    const style = getComputedStyle(el);
    if (el.style.display === 'none' || style.display === 'none' || style.visibility === 'hidden') return false;
    // AMUX's older overlay primitive hides with pointer-events/opacity rather
    // than display:none. Treat it as closed for trigger and accessibility state.
    if (style.pointerEvents === 'none' && Number(style.opacity || 1) === 0) return false;
    if (style.pointerEvents === 'none' && !el.classList.contains('show')) return false;
    return true;
  }

  function syncTriggers() {
    for (const pair of TRIGGER_PAIRS) {
      const trigger = document.getElementById(pair[0]);
      const content = document.getElementById(pair[1]);
      if (!trigger || !content) continue;
      setAttr(trigger, 'aria-haspopup', content.matches('[role=menu]') ? 'menu' : 'dialog');
      setAttr(trigger, 'aria-controls', content.id);
      setAttr(trigger, 'aria-expanded', isShown(content) ? 'true' : 'false');
    }
    for (const menu of document.querySelectorAll('.card-menu[id]')) {
      const card = menu.closest('.card');
      const trigger = card && card.querySelector('.card-menu-btn');
      if (!trigger) continue;
      setAttr(trigger, 'aria-haspopup', 'menu');
      setAttr(trigger, 'aria-controls', menu.id);
      setAttr(trigger, 'aria-expanded', menu.classList.contains('open') ? 'true' : 'false');
    }
  }

  function enhance(root) {
    const scope = root && root.querySelectorAll ? root : document;
    enhanceViews(document);
    enhanceTabs(scope);
    enhanceToggleGroups();
    enhanceMenus(scope);
    enhanceButtons(scope);
    enhanceFields(scope);
    enhanceSurfaces(scope);
    enhanceDialogs(scope);
    syncTriggers();
    if (window.basecoat && typeof window.basecoat.initAll === 'function') {
      window.basecoat.initAll();
    }
  }

  let scheduled = false;
  function scheduleEnhance() {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(() => {
      scheduled = false;
      enhance(document);
    });
  }

  document.documentElement.classList.add('dark');
  document.documentElement.setAttribute('data-ui-system', 'basecoat');
  window.AmuxUI = { enhance, sync: scheduleEnhance };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => enhance(document), { once: true });
  } else {
    enhance(document);
  }

  const observer = new MutationObserver(scheduleEnhance);
  observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['class', 'style', 'hidden'],
  });
})();
