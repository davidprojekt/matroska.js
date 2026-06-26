// A small dropdown menu for the video.js control bar: a trigger button plus a popup
// list, styled with the skin's `media-surface`/`media-button` classes. Self-contained
// (no dependency on the library's internal menu/popover components) — it just renders
// option rows and reports the chosen value.

export class TrackMenu {
  /**
   * @param trigger  the <button> in the control bar that opens the menu
   * @param popup    the popup container element (role=menu)
   * @param onSelect (value) => void, called when an item is chosen
   */
  constructor(trigger, popup, onSelect) {
    this.trigger = trigger;
    this.popup = popup;
    this.onSelect = onSelect;
    this.value = null;

    trigger.addEventListener('click', () => this.toggle());
    // Close on outside click / Escape. We do NOT stopPropagation on the trigger, so a
    // click on one trigger bubbles to the document and closes any other open menu —
    // letting only one menu be open at a time. The clicked trigger's own listener sees
    // `target === this.trigger` and leaves itself alone.
    document.addEventListener('click', (e) => {
      if (this.isOpen() && !this.popup.contains(e.target) && e.target !== this.trigger) {
        this.close();
      }
    });
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && this.isOpen()) this.close();
    });
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
   * @param items [{ value, label, disabled?, selected? }]
   */
  setItems(items) {
    this.popup.textContent = '';
    this.value = null;
    for (const item of items) {
      const row = document.createElement('button');
      row.type = 'button';
      row.className = 'vjs-menu__item';
      row.setAttribute('role', 'menuitemradio');
      row.textContent = item.label;
      row.dataset.value = item.value;
      if (item.disabled) row.disabled = true;
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

  /** Reflect a value as the checked item (without firing onSelect). */
  setValue(value) {
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
