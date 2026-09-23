/** The explorer and compact picker share the existing example selection path. */
export function createExampleBrowser(
  host: HTMLElement,
  select: HTMLSelectElement,
  sourceTitle: HTMLElement,
) {
  const buttons = new Map<string, HTMLButtonElement>();

  function sync(): void {
    for (const [value, button] of buttons) {
      const selected = value === select.value;
      button.classList.toggle("selected", selected);
      if (selected) {
        button.setAttribute("aria-current", "true");
        const group = button.closest("details");
        if (group) group.open = true;
      } else {
        button.removeAttribute("aria-current");
      }
    }
    const value = select.value;
    sourceTitle.textContent = !value ? "Untitled.fr"
      : value === "__custom__" ? "Custom source"
      : `${value.split("/").at(-1)}.fr`;
    sourceTitle.title = value && value !== "__custom__" ? `${value}.fr` : sourceTitle.textContent;
  }

  function addOption(option: HTMLOptionElement, parent: HTMLElement): void {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "example-file";
    button.dataset.example = option.value;
    button.textContent = option.textContent?.trim() || "Untitled";
    button.title = option.value || "New blank source";
    button.addEventListener("click", () => {
      select.value = option.value;
      select.dispatchEvent(new Event("change", { bubbles: true }));
      sync();
    });
    buttons.set(option.value, button);
    parent.appendChild(button);
  }

  function populate(): void {
    host.replaceChildren();
    buttons.clear();
    for (const child of Array.from(select.children)) {
      if (child instanceof HTMLOptGroupElement) {
        const group = document.createElement("details");
        const summary = document.createElement("summary");
        summary.textContent = child.label;
        group.appendChild(summary);
        for (const option of Array.from(child.children)) {
          if (option instanceof HTMLOptionElement) addOption(option, group);
        }
        host.appendChild(group);
      } else if (child instanceof HTMLOptionElement) {
        addOption(child, host);
      }
    }
    sync();
  }

  select.addEventListener("change", sync);
  return { populate, sync };
}
