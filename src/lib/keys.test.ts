import { describe, expect, it } from "vitest";
import { isFromNestedControl, isOwnActivationControl } from "./keys";

function rowWithButton() {
  const row = document.createElement("div");
  row.setAttribute("role", "button");
  row.dataset.prRow = "acme/app#1";
  row.innerHTML = '<span class="title">Title</span><button type="button"><svg><path /></svg></button>';
  document.body.append(row);
  return {
    row,
    title: row.querySelector(".title") as HTMLElement,
    button: row.querySelector("button") as HTMLButtonElement,
    path: row.querySelector("path") as Element,
  };
}

describe("isOwnActivationControl", () => {
  it("is true for a button inside a row and false for the row itself", () => {
    const { row, button } = rowWithButton();
    expect(isOwnActivationControl(button)).toBe(true);
    expect(isOwnActivationControl(row)).toBe(false);
    row.remove();
  });
});

describe("isFromNestedControl", () => {
  it("is true for a click on a button inside the row, or on the button's icon", () => {
    const { row, button, path } = rowWithButton();
    expect(isFromNestedControl(button, row)).toBe(true);
    expect(isFromNestedControl(path, row)).toBe(true);
    row.remove();
  });

  it("is false for a click on the row's own content or the row itself", () => {
    const { row, title } = rowWithButton();
    expect(isFromNestedControl(title, row)).toBe(false);
    expect(isFromNestedControl(row, row)).toBe(false);
    expect(isFromNestedControl(null, row)).toBe(false);
    row.remove();
  });

  it("is true for a menu item that lives outside the row", () => {
    const { row } = rowWithButton();
    const item = document.createElement("button");
    item.setAttribute("role", "menuitem");
    document.body.append(item);
    expect(isFromNestedControl(item, row)).toBe(true);
    row.remove();
    item.remove();
  });
});
