// Stacked pull requests in the list: a PR stacked on another listed PR sits
// under it, with a ↳ arrow and its title block stepped in. The mock stacks
// dashpay/platform #5750 on #5749 and #5967 on #5750.

import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { createAppStore, StoreProvider } from "../../state/store";
import { PrTable } from "./PrTable";

async function setup() {
  const api = createMockApi({ latencyMs: 0 });
  const store = createAppStore(api);
  await store.getState().init();
  render(
    <StoreProvider store={store}>
      <PrTable />
    </StoreProvider>,
  );
  return { store };
}

const P = "dashpay/platform#";
const rows = () => [...document.querySelectorAll<HTMLElement>("[data-pr-row]")];
const rowIds = () => rows().map((r) => r.dataset.prRow as string);
const row = (id: string) => rows().find((r) => r.dataset.prRow === id) as HTMLElement;
const main = (id: string) => row(id).querySelector(".pr-row__main") as HTMLElement;

describe("Stacked rows", () => {
  it("list a stack under its parent, in the order shown", async () => {
    await setup();
    const ids = rowIds();
    const at = ids.indexOf(`${P}5749`);
    expect(ids.slice(at, at + 3)).toEqual([`${P}5749`, `${P}5750`, `${P}5967`]);
  });

  it("start the title line with an arrow named after the parent, part of the row's name", async () => {
    await setup();
    const arrow = row(`${P}5750`).querySelector(".pr-row__title > :first-child") as HTMLElement;
    expect(arrow).toHaveClass("pr-row__stacked");
    expect(arrow).toHaveTextContent("↳");
    expect(arrow).toHaveAttribute("role", "img");
    expect(arrow).toHaveAccessibleName("Stacked on #5749");
    expect(arrow).toHaveAttribute("title", "Stacked on #5749: feat!: sys.random.uniform document properties");
    // The test library trims each child's text, so the space after the comma is lost here.
    expect(row(`${P}5750`)).toHaveAccessibleName(/^Stacked on #5749,\s*feat\(platform\)!: random token/);
    expect(screen.getByRole("img", { name: "Stacked on #5750" })).toBeInTheDocument();
  });

  it("step the title block in by depth, and leave other rows alone", async () => {
    await setup();
    expect(row(`${P}5750`)).toHaveClass("stacked");
    expect(main(`${P}5750`).dataset.stackDepth).toBe("1");
    expect(main(`${P}5750`).style.getPropertyValue("--stack-depth")).toBe("1");
    expect(main(`${P}5967`).dataset.stackDepth).toBe("2");
    expect(main(`${P}5967`).style.getPropertyValue("--stack-depth")).toBe("2");
    // The parent and unrelated rows: no arrow, no step.
    for (const id of [`${P}5749`, `${P}6033`]) {
      expect(row(id)).not.toHaveClass("stacked");
      expect(row(id).querySelector(".pr-row__stacked")).toBeNull();
      expect(main(id).dataset.stackDepth).toBeUndefined();
    }
    // The step is on the title block only: the status, time and CI cells sit outside it.
    expect(main(`${P}5750`).querySelector(".pr-row__status, .pr-row__time, .pr-row__ci")).toBeNull();
  });

  it("moves the keyboard cursor down the stack in the order shown", async () => {
    const user = userEvent.setup();
    const { store } = await setup();
    const ids = rowIds();
    const at = ids.indexOf(`${P}5749`);
    store.getState().setFocused(`${P}5749`);
    await user.keyboard("j");
    expect(store.getState().focusedId).toBe(`${P}5750`);
    await user.keyboard("j");
    expect(store.getState().focusedId).toBe(`${P}5967`);
    await user.keyboard("j");
    expect(store.getState().focusedId).toBe(ids[at + 3] ?? `${P}5967`);
    await user.keyboard("kkk");
    expect(store.getState().focusedId).toBe(`${P}5749`);
  });

  it("are not nested when grouped by status, which puts parent and child apart", async () => {
    const user = userEvent.setup();
    await setup();
    await user.click(screen.getByRole("button", { name: "Status" }));
    expect(document.querySelector(".pr-row__stacked")).toBeNull();
    expect(rowIds()).toContain(`${P}5750`);
    expect(rowIds()).toContain(`${P}5967`);
  });
});
