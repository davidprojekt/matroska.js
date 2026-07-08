// A small dropdown menu for the video.js control bar: a trigger button plus a popup
// list, styled with the skin's `media-surface`/`media-button` classes. Self-contained
// (no dependency on the library's internal menu/popover components) — it just renders
// option rows and reports the chosen value.
//
// Two modes:
//   • single-select (default) — audio/chapters: one checked value, chosen row fires
//     onSelect(value) and closes the menu.
//   • multi-select — subtitles: up to `maxSelect` values active at once. Each `addable`
//     row has two hit areas: the main label and a trailing add/remove (+/✓) button.
//     Below the cap, clicking the label *replaces* the selection with that one track
//     while the add button *adds* it alongside the others; once the cap is reached every
//     click is a plain toggle (and adding a further track is disabled). The "Off" row
//     (value '') clears everything. onSelect receives the current array of values and the
//     menu stays open so a second track can be picked.

export class TrackMenu {
  /**
   * @param trigger  the <button> in the control bar that opens the menu
   * @param popup    the popup container element (role=menu)
   * @param onSelect single-select: (value) => void; multi-select: (values[]) => void
   * @param opts     { multiSelect?: boolean, maxSelect?: number }
   */
  constructor(trigger, popup, onSelect, opts = {}) {
    this.trigger = trigger;
    this.popup = popup;
    this.onSelect = onSelect;
    this.multiSelect = !!opts.multiSelect;
    this.maxSelect = opts.maxSelect ?? Infinity;
    this.value = null; // single-select: the checked value
    this.values = new Set(); // multi-select: the active values (order irrelevant)

    this._onTriggerClick = () => this.toggle();
    trigger.addEventListener('click', this._onTriggerClick);
    // Close on outside click / Escape. We do NOT stopPropagation on the trigger, so a
    // click on one trigger bubbles to the document and closes any other open menu —
    // letting only one menu be open at a time. The clicked trigger's own listener sees
    // `target === this.trigger` and leaves itself alone. The document listeners are kept
    // on `this` so destroy() can detach them (a player can be torn down and rebuilt).
    this._onDocClick = (e) => {
      if (this.isOpen() && !this.popup.contains(e.target) && e.target !== this.trigger) {
        this.close();
      }
    };
    this._onDocKeydown = (e) => {
      if (e.key === 'Escape' && this.isOpen()) this.close();
    };
    document.addEventListener('click', this._onDocClick);
    document.addEventListener('keydown', this._onDocKeydown);
  }

  /** Detach the document-level listeners. Call when tearing the player down. */
  destroy() {
    this.trigger.removeEventListener('click', this._onTriggerClick);
    document.removeEventListener('click', this._onDocClick);
    document.removeEventListener('keydown', this._onDocKeydown);
  }

  isOpen() {
    return !this.popup.hidden;
  }
  open() {
    this.popup.hidden = false;
    this.trigger.setAttribute('aria-expanded', 'true');
  }
  close() {
    this.popup.hidden = true;
    this.trigger.setAttribute('aria-expanded', 'false');
  }
  toggle() {
    this.isOpen() ? this.close() : this.open();
  }

  /**
   * Rebuild the option list.
   * @param items [{ value, label, disabled?, selected?, addable? }]
   *   selected — single-select: the initially-checked row; multi-select: seeds `values`.
   *   addable  — multi-select only: render the +/✓ add button (ASS tracks). The "Off" row
   *              and disabled rows omit it.
   */
  setItems(items) {
    this.popup.textContent = '';
    if (this.multiSelect) {
      this.values = new Set(items.filter((i) => i.selected).map((i) => String(i.value)));
      this._items = items;
      this._render();
      return;
    }
    this.value = null;
    for (const item of items) {
      const row = this._makeItemButton(item);
      const selected = !!item.selected;
      row.setAttribute('aria-checked', String(selected));
      if (selected) this.value = item.value;
      if (!item.disabled) {
        row.addEventListener('click', () => {
          this.setValue(item.value);
          this.close();
          this.onSelect(item.value);
        });
      }
      this.popup.appendChild(row);
    }
  }

  // --- multi-select internals ---

  _makeItemButton(item) {
    const row = document.createElement('button');
    row.type = 'button';
    row.className = 'vjs-menu__item';
    row.setAttribute('role', this.multiSelect ? 'menuitemcheckbox' : 'menuitemradio');
    row.textContent = item.label;
    row.dataset.value = item.value;
    if (item.disabled) row.disabled = true;
    return row;
  }

  // Render (multi-select) rows from `this._items`, reflecting the current `values` set.
  _render() {
    const atMax = this.values.size >= this.maxSelect;
    this.popup.textContent = '';
    for (const item of this._items) {
      const value = String(item.value);
      const active = this.values.has(value);
      const isOff = value === '';

      const main = this._makeItemButton(item);
      main.setAttribute('aria-checked', String(isOff ? this.values.size === 0 : active));

      if (item.addable && !item.disabled) {
        // Two-hit-area row: label (main) + add/remove button.
        const rowEl = document.createElement('div');
        rowEl.className = 'vjs-menu__row';
        main.classList.add('vjs-menu__item--main');

        const add = document.createElement('button');
        add.type = 'button';
        add.className = 'vjs-menu__add';
        add.dataset.value = value;
        add.setAttribute('aria-pressed', String(active));
        add.textContent = active ? '−' : '+';
        add.title = active ? 'Remove this subtitle' : 'Add as second subtitle';
        // Below the cap the add button adds alongside; at the cap it can only remove, so
        // it's disabled on inactive rows (can't exceed maxSelect).
        add.disabled = atMax && !active;

        // stopPropagation: _apply() rebuilds the popup, detaching this node, so the
        // document click-handler would otherwise see the click as "outside" and close us.
        main.addEventListener('click', (e) => {
          e.stopPropagation();
          this._onMainClick(value, atMax);
        });
        if (!add.disabled)
          add.addEventListener('click', (e) => {
            e.stopPropagation();
            this._onAddClick(value);
          });

        rowEl.append(main, add);
        this.popup.appendChild(rowEl);
      } else {
        // "Off" and disabled rows: single button, whole-row click.
        if (!item.disabled) {
          main.addEventListener('click', (e) => {
            e.stopPropagation();
            if (isOff) this._apply(new Set());
            else this._onMainClick(value, atMax);
          });
        }
        this.popup.appendChild(main);
      }
    }
  }

  // Label click. Below the cap: replace the selection with just this track. At the cap:
  // toggle (remove if active; inactive rows are a no-op — they're greyed to add anyway).
  _onMainClick(value, atMax) {
    if (atMax) {
      if (this.values.has(value)) {
        const next = new Set(this.values);
        next.delete(value);
        this._apply(next);
      }
      return;
    }
    this._apply(new Set([value]));
  }

  // Add-button click (only reachable below the cap, or on an active row at the cap):
  // toggle this track's membership, keeping the others.
  _onAddClick(value) {
    const next = new Set(this.values);
    next.has(value) ? next.delete(value) : next.add(value);
    this._apply(next);
  }

  _apply(nextSet) {
    this.values = nextSet;
    this._render();
    this.onSelect([...this.values]);
  }

  /**
   * Reflect a value (single-select) or an array of values (multi-select) as checked,
   * without firing onSelect.
   */
  setValue(value) {
    if (this.multiSelect) {
      const arr = Array.isArray(value) ? value : value === '' || value == null ? [] : [value];
      this.values = new Set(arr.map(String));
      if (this._items) this._render();
      return;
    }
    this.value = value;
    for (const row of this.popup.querySelectorAll('.vjs-menu__item')) {
      row.setAttribute('aria-checked', String(row.dataset.value === String(value)));
    }
  }

  /** Show/hide the whole control depending on whether it has any usable items. */
  setAvailable(available) {
    this.trigger.hidden = !available;
  }
}
