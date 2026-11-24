import { describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMockApi } from "../../mock/mockApi";
import { mockSnapshot } from "../../mock/snapshot";
import { createAppStore, StoreProvider } from "../../state/store";
import type { Api } from "../../api";
import type { PrTrait, PullRequest, Snapshot } from "../../types";
import { PrTable } from "./PrTable";

const GROVE_URL = "https://dashpay.github.io/grovedb-structure-viewer/?base=083c865a&head=dashpay%2Fplatform%405acaa407";
const GROVE: PrTrait = {
  kind: "grovedbChange",
  summary: "5 nodes changed",
  links: [{ label: "Structure viewer", url: GROVE_URL }],
};
const ONE_CONTRACT: PrTrait = {
  kind: "contractChange",
  summary: "token-history-contract v3: new version",
  links: [{ label: "token-history-contract v3", url: "https://example.test/contract?c=token-history&v=3" }],
};
const THREE_CONTRACTS: PrTrait = {
  kind: "contractChange",
  summary: "3 system contracts changed",
  links: [
    { label: "dpns-contract v2 (compare with v1)", url: "https://example.test/contract?c=dpns" },
    { label: "dashpay-contract v2 (compare with v1)", url: "https://example.test/contract?c=dashpay" },
    { label: "token-history-contract v3 (compare with v2)", url: "https://example.test/contract?c=token-history" },
  ],
};
const NO_LINK: PrTrait = { kind: "grovedbChange", summary: "Structure description changed", links: [] };

/** One project, one PR per entry, each with the traits given. */
function snapshotWith(traitLists: (PrTrait[] | undefined)[]): Snapshot {
  const base = mockSnapshot();
  const template = base.projects[0].prs[0];
  const prs: PullRequest[] = traitLists.map((traits, i) => {
    const n = i + 1;
    const pr: PullRequest = {
      ...template,
      id: `acme/app#${n}`,
      repo: "acme/app",
      number: n,
      title: `PR ${n}`,
      url: `https://github.com/acme/app/pull/${n}`,
      bot: { state: "approved", at: null },
      traits: traits ?? [],
    };
    // An older backend sends no `traits` at all.
    if (traits === undefined) delete (pr as Partial<PullRequest>).traits;
    return pr;
  });
  return { ...base, projects: [{ repo: "acme/app", openCount: prs.length, prs, error: null }] };
}

async function setup(snapshot: Snapshot) {
  const api: Api = createMockApi({ latencyMs: 0, snapshot });
  const openUrl = vi.fn(async () => {});
  api.openUrl = openUrl;
  const store = createAppStore(api);
  await store.getState().init();
  render(
    <StoreProvider store={store}>
      <PrTable />
    </StoreProvider>,
  );
  return { store, openUrl };
}

const row = (n: number) => document.querySelector<HTMLElement>(`[data-pr-row="acme/app#${n}"]`) as HTMLElement;
const traitCell = (n: number) => row(n).querySelector<HTMLElement>(".pr-row__traits") as HTMLElement;
const kinds = (n: number) => [...traitCell(n).querySelectorAll<HTMLElement>("[data-trait]")].map((e) => e.dataset.trait);

describe("trait icons in the row", () => {
  it("shows an icon for each trait present, GroveDB first, with names and tooltips", async () => {
    await setup(snapshotWith([[ONE_CONTRACT, GROVE], [ONE_CONTRACT], []]));
    expect(kinds(1)).toEqual(["grovedbChange", "contractChange"]);
    expect(kinds(2)).toEqual(["contractChange"]);
    expect(kinds(3)).toEqual([]);

    const grove = within(row(1)).getByRole("button", {
      name: "GroveDB change: 5 nodes changed. Open the structure viewer",
    });
    expect(grove).toHaveAttribute("title", "GroveDB change: 5 nodes changed. Open the structure viewer");
    const contract = within(row(2)).getByRole("button", {
      name: "Contract change: token-history-contract v3: new version. Open the contract viewer",
    });
    expect(contract).toHaveAttribute("title", contract.getAttribute("aria-label"));
    expect(contract.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
  });

  it("leaves the column out when no listed row has a trait", async () => {
    await setup(snapshotWith([[], []]));
    const scroll = document.querySelector<HTMLElement>(".pr-table__scroll") as HTMLElement;
    expect(scroll).not.toHaveClass("has-traits");
  });

  it("sizes the column for the row with the most traits", async () => {
    await setup(snapshotWith([[GROVE], [GROVE, ONE_CONTRACT], []]));
    const scroll = document.querySelector<HTMLElement>(".pr-table__scroll") as HTMLElement;
    expect(scroll).toHaveClass("has-traits");
    expect(scroll.style.getPropertyValue("--trait-slots")).toBe("2");
  });

  it("opens a single link on click without expanding the row", async () => {
    const user = userEvent.setup();
    const { store, openUrl } = await setup(snapshotWith([[GROVE]]));
    await user.click(within(row(1)).getByRole("button", { name: /^GroveDB change/ }));
    expect(openUrl).toHaveBeenCalledWith(GROVE_URL);
    expect(store.getState().expandedId).toBeNull();
    expect(row(1)).toHaveAttribute("aria-expanded", "false");
  });

  it("opens a single link with Enter and with Space without expanding the row", async () => {
    const user = userEvent.setup();
    const { store, openUrl } = await setup(snapshotWith([[GROVE]]));
    const icon = within(row(1)).getByRole("button", { name: /^GroveDB change/ });
    icon.focus();
    await user.keyboard("{Enter}");
    expect(openUrl).toHaveBeenCalledTimes(1);
    expect(openUrl).toHaveBeenLastCalledWith(GROVE_URL);
    expect(store.getState().expandedId).toBeNull();
    expect(icon).toHaveFocus();

    await user.keyboard(" ");
    expect(openUrl).toHaveBeenCalledTimes(2);
    expect(store.getState().expandedId).toBeNull();
  });

  it("still expands the row on a click elsewhere in it", async () => {
    const user = userEvent.setup();
    const { store, openUrl } = await setup(snapshotWith([[GROVE]]));
    await user.click(within(row(1)).getByText("PR 1"));
    expect(store.getState().expandedId).toBe("acme/app#1");
    expect(openUrl).not.toHaveBeenCalled();
  });

  it("opens a menu of links for a trait with several, opens the chosen one, and closes on Esc", async () => {
    const user = userEvent.setup();
    const { store, openUrl } = await setup(snapshotWith([[THREE_CONTRACTS]]));
    const icon = within(row(1)).getByRole("button", {
      name: "Contract change: 3 system contracts changed. Choose one of 3 links to open",
    });
    expect(icon).toHaveAttribute("aria-haspopup", "menu");
    expect(icon).toHaveAttribute("aria-expanded", "false");

    await user.click(icon);
    let menu = screen.getByRole("menu", { name: "Contract change: 3 system contracts changed" });
    expect(icon).toHaveAttribute("aria-expanded", "true");
    expect(openUrl).not.toHaveBeenCalled();
    expect(store.getState().expandedId).toBeNull();
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual(THREE_CONTRACTS.links.map((l) => l.label));
    expect(items[0]).toHaveFocus();

    // Arrow keys move inside the menu and do not move through the list.
    await user.keyboard("{ArrowDown}");
    expect(items[1]).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(icon).toHaveFocus();
    expect(store.getState().expandedId).toBeNull();

    await user.click(icon);
    menu = screen.getByRole("menu");
    await user.click(within(menu).getByRole("menuitem", { name: "dashpay-contract v2 (compare with v1)" }));
    expect(openUrl).toHaveBeenCalledWith("https://example.test/contract?c=dashpay");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(store.getState().expandedId).toBeNull();
  });

  it("chooses a menu item with Enter without expanding the row", async () => {
    const user = userEvent.setup();
    const { store, openUrl } = await setup(snapshotWith([[THREE_CONTRACTS]]));
    within(row(1)).getByRole("button", { name: /^Contract change/ }).focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.keyboard("{End}{Enter}");
    expect(openUrl).toHaveBeenCalledWith("https://example.test/contract?c=token-history");
    expect(store.getState().expandedId).toBeNull();
  });

  it("closes the menu on a click outside", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([[THREE_CONTRACTS], []]));
    await user.click(within(row(1)).getByRole("button", { name: /^Contract change/ }));
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.click(document.body);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("shows a trait with no links as a picture, not a button", async () => {
    const user = userEvent.setup();
    const { store, openUrl } = await setup(snapshotWith([[NO_LINK]]));
    expect(within(row(1)).queryByRole("button", { name: /GroveDB/ })).not.toBeInTheDocument();
    const img = within(row(1)).getByRole("img", { name: "GroveDB change: Structure description changed" });
    expect(img).toHaveAttribute("title", "GroveDB change: Structure description changed");
    await user.click(img);
    expect(openUrl).not.toHaveBeenCalled();
    // A click on the picture is a click on the row.
    expect(store.getState().expandedId).toBe("acme/app#1");
  });

  it("puts trait icons in the Tab order only in the row that holds the Tab stop", async () => {
    await setup(snapshotWith([[GROVE], [GROVE]]));
    expect(within(row(1)).getByRole("button", { name: /^GroveDB/ })).toHaveAttribute("tabindex", "0");
    expect(within(row(2)).getByRole("button", { name: /^GroveDB/ })).toHaveAttribute("tabindex", "-1");
  });

  it("renders a snapshot from a backend that sends no traits", async () => {
    await setup(snapshotWith([undefined, undefined]));
    expect(row(1)).toBeInTheDocument();
    expect(kinds(1)).toEqual([]);
    expect(document.querySelector(".pr-table__scroll")).not.toHaveClass("has-traits");
  });
});

describe("traits in the detail", () => {
  async function openDetail(user: ReturnType<typeof userEvent.setup>, n: number) {
    await user.click(within(row(n)).getByText(`PR ${n}`));
    return screen.getByRole("region", { name: `Details for #${n}` });
  }
  const traitsFact = (detail: HTMLElement) =>
    within(detail).queryByText("Traits", { selector: "dt" })?.closest<HTMLElement>(".pr-fact") ?? null;

  it("lists each trait with its summary and every link", async () => {
    const user = userEvent.setup();
    const { openUrl } = await setup(snapshotWith([[GROVE, THREE_CONTRACTS]]));
    const fact = traitsFact(await openDetail(user, 1));
    expect(fact).not.toBeNull();
    const items = within(fact as HTMLElement).getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("GroveDB change");
    expect(items[0]).toHaveTextContent("5 nodes changed");
    expect(items[1]).toHaveTextContent("Contract change");
    expect(items[1]).toHaveTextContent("3 system contracts changed");
    const links = within(items[1]).getAllByRole("button");
    expect(links.map((b) => b.textContent)).toEqual(THREE_CONTRACTS.links.map((l) => l.label));

    await user.click(within(items[0]).getByRole("button", { name: "Structure viewer" }));
    expect(openUrl).toHaveBeenLastCalledWith(GROVE_URL);
    await user.click(links[2]);
    expect(openUrl).toHaveBeenLastCalledWith("https://example.test/contract?c=token-history");
    // The detail stays open.
    expect(screen.getByRole("region", { name: "Details for #1" })).toBeInTheDocument();
  });

  it("says when a trait has no link", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([[NO_LINK]]));
    const fact = traitsFact(await openDetail(user, 1)) as HTMLElement;
    expect(fact).toHaveTextContent("Structure description changed");
    expect(fact).toHaveTextContent("No viewer link");
    expect(within(fact).queryByRole("button")).not.toBeInTheDocument();
  });

  it("leaves the fact out when there are no traits, or no traits field", async () => {
    const user = userEvent.setup();
    await setup(snapshotWith([[], undefined]));
    expect(traitsFact(await openDetail(user, 1))).toBeNull();
    expect(traitsFact(await openDetail(user, 2))).toBeNull();
  });
});
