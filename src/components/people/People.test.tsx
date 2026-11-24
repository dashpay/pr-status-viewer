import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { Api } from "../../api";
import { mockConfig } from "../../mock/snapshot";
import type { KnownContributor } from "../../types";
import { makeApi, renderShell } from "../shell/testUtils";
import { PrTable } from "../table/PrTable";

const switcher = () => screen.getByRole("button", { name: /^Whose pull requests: / });
const menu = () => screen.getByRole("menu", { name: "Whose pull requests" });
const heading = () => screen.getByRole("heading", { level: 1 });
const dialog = () => screen.getByRole("dialog", { name: "Track contributor" });
const field = () => within(dialog()).getByRole("textbox", { name: "GitHub username" });
const rowIds = () => [...document.querySelectorAll<HTMLElement>("[data-pr-row]")].map((r) => r.dataset.prRow);

function withContributors(contributors: string[]): Api {
  return makeApi({ config: { ...mockConfig(), contributors } });
}

async function render(api: Api = makeApi()) {
  return renderShell(api, { children: <PrTable /> });
}

async function openDialog(user: ReturnType<typeof userEvent.setup>) {
  await user.click(switcher());
  await user.click(within(menu()).getByRole("menuitem", { name: "Track contributor…" }));
  return dialog();
}

describe("person switcher", () => {
  it("reads Mine in the user's own views", async () => {
    const user = userEvent.setup();
    const { store } = await render();
    expect(switcher()).toHaveAccessibleName("Whose pull requests: Mine");
    for (const view of [{ kind: "following" }, { kind: "needsMe" }, { kind: "project", repo: "dashpay/dips" }] as const) {
      act(() => store.getState().setView(view));
      expect(switcher()).toHaveTextContent("Mine");
    }
    await user.click(switcher());
    expect(switcher()).toHaveAttribute("aria-expanded", "true");
    const mine = within(menu()).getByRole("menuitemradio", { name: /^Mine/ });
    expect(mine).toHaveAttribute("aria-checked", "true");
    expect(mine).toHaveTextContent("11, 11 open pull requests");
    expect(within(menu()).getByRole("separator")).toBeInTheDocument();
    expect(within(menu()).getByRole("menuitem", { name: "Track contributor…" })).toBeInTheDocument();
    expect(within(menu()).queryByRole("menuitem", { name: /^Stop tracking/ })).toBeNull();
  });

  it("lists tracked contributors in order with their counts, and switches by keyboard", async () => {
    const user = userEvent.setup();
    const { store } = await render(withContributors(["tidepools", "OrbitWren"]));
    await user.click(switcher());
    const radios = within(menu()).getAllByRole("menuitemradio");
    expect(radios.map((r) => r.textContent)).toEqual([
      "✓Mine11, 11 open pull requests",
      "tidepools2, 2 open pull requests",
      "OrbitWren3, 3 open pull requests",
    ]);
    expect(radios[0]).toHaveFocus();
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");

    expect(store.getState().view).toEqual({ kind: "contributor", login: "OrbitWren" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(switcher()).toHaveFocus();
    expect(switcher()).toHaveAccessibleName("Whose pull requests: OrbitWren");
    expect(heading()).toHaveTextContent("OrbitWren's pull requests");

    await user.click(switcher());
    expect(within(menu()).getByRole("menuitemradio", { name: /^OrbitWren/ })).toHaveAttribute("aria-checked", "true");
    expect(within(menu()).getByRole("menuitemradio", { name: /^Mine/ })).toHaveAttribute("aria-checked", "false");
    await user.click(within(menu()).getByRole("menuitemradio", { name: /^Mine/ }));
    expect(store.getState().view).toEqual({ kind: "mine" });
    expect(heading()).toHaveTextContent("Mine");
  });

  it("closes on Esc and gives focus back", async () => {
    const user = userEvent.setup();
    await render();
    await user.click(switcher());
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(switcher()).toHaveFocus();
  });

  it("titles a login ending in s, and truncates with the full name in a title", async () => {
    const { store } = await render(withContributors(["tidepools", "northwind-automation-collective"]));
    act(() => store.getState().setView({ kind: "contributor", login: "tidepools" }));
    expect(heading()).toHaveTextContent("tidepools' pull requests");
    act(() => store.getState().setView({ kind: "contributor", login: "northwind-automation-collective" }));
    expect(heading()).toHaveAttribute("title", "northwind-automation-collective's pull requests");
    expect(switcher()).toHaveAttribute("title", "Showing northwind-automation-collective's pull requests");
  });

  it("stops tracking the contributor in view and goes back to Mine", async () => {
    const user = userEvent.setup();
    const { store } = await render(withContributors(["OrbitWren", "LumenAsh"]));
    act(() => store.getState().setView({ kind: "contributor", login: "OrbitWren" }));
    expect(rowIds()).toHaveLength(3);
    await user.click(switcher());
    await user.click(within(menu()).getByRole("menuitem", { name: "Stop tracking OrbitWren" }));

    await waitFor(() => expect(store.getState().view).toEqual({ kind: "mine" }));
    expect(store.getState().config?.contributors).toEqual(["LumenAsh"]);
    expect(switcher()).toHaveAccessibleName("Whose pull requests: Mine");
    expect(switcher()).toHaveFocus();
    expect(store.getState().snapshot?.projects.flatMap((p) => p.prs).some((pr) => pr.author === "OrbitWren")).toBe(false);
    await user.click(switcher());
    expect(within(menu()).queryByRole("menuitemradio", { name: /^OrbitWren/ })).toBeNull();
    expect(within(menu()).queryByRole("menuitem", { name: /^Stop tracking/ })).toBeNull();
  });

  it("says so when stopping fails", async () => {
    const user = userEvent.setup();
    const api = makeApi(
      { config: { ...mockConfig(), contributors: ["OrbitWren"] } },
      { untrackContributor: () => Promise.reject("Could not save your settings.") },
    );
    const { store } = await render(api);
    act(() => store.getState().setView({ kind: "contributor", login: "OrbitWren" }));
    await user.click(switcher());
    await user.click(within(menu()).getByRole("menuitem", { name: "Stop tracking OrbitWren" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not stop tracking OrbitWren: Could not save your settings.",
    );
    expect(store.getState().view).toEqual({ kind: "contributor", login: "OrbitWren" });
  });
});

describe("contributor view", () => {
  it("lists exactly that author's pull requests, with the table's filters", async () => {
    const { store } = await render(withContributors(["OrbitWren", "kestrel-dev"]));
    act(() => store.getState().setView({ kind: "contributor", login: "orbitwren" }));
    expect(rowIds().sort()).toEqual([
      "dashpay/dash-evo-tool#1226",
      "dashpay/platform#6041",
      "dashpay/platform#6044",
    ]);
    expect(screen.getByText("3 open")).toBeInTheDocument();
    // The drafts switch works here as in Mine.
    act(() => store.getState().setShowDrafts(false));
    expect(rowIds()).not.toContain("dashpay/dash-evo-tool#1226");
  });

  it("explains an empty list", async () => {
    const { store } = await render(withContributors(["OrbitWren"]));
    act(() => store.getState().setView({ kind: "contributor", login: "someone-else" }));
    expect(screen.getByRole("status")).toHaveTextContent(
      "someone-else has no open pull requests in the projects you track.",
    );
  });
});

describe("Track contributor dialog", () => {
  it("tracks a typed name in any casing, switches to that person and closes", async () => {
    const user = userEvent.setup();
    const { store } = await render();
    await openDialog(user);
    expect(field()).toHaveFocus();
    await user.type(field(), "orbitwren{Enter}");

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(store.getState().config?.contributors).toEqual(["OrbitWren"]);
    expect(store.getState().view).toEqual({ kind: "contributor", login: "OrbitWren" });
    expect(heading()).toHaveTextContent("OrbitWren's pull requests");
    expect(rowIds()).toHaveLength(3);
    expect(switcher()).toHaveFocus();
  });

  it("accepts @login and a profile link", async () => {
    const user = userEvent.setup();
    const { store } = await render();
    await openDialog(user);
    await user.type(field(), "@Kestrel-Dev");
    await user.click(within(dialog()).getByRole("button", { name: "Track" }));
    await waitFor(() => expect(store.getState().view).toEqual({ kind: "contributor", login: "kestrel-dev" }));

    await openDialog(user);
    await user.type(field(), "https://github.com/LumenAsh{Enter}");
    await waitFor(() => expect(store.getState().view).toEqual({ kind: "contributor", login: "LumenAsh" }));
    expect(store.getState().config?.contributors).toEqual(["kestrel-dev", "LumenAsh"]);
  });

  it("shows a rejected name next to the field and stays open", async () => {
    const user = userEvent.setup();
    const { store } = await render();
    await openDialog(user);
    await user.type(field(), "no-such-person{Enter}");
    const alert = await within(dialog()).findByRole("alert");
    expect(alert).toHaveTextContent("There is no GitHub user named no-such-person.");
    expect(field()).toHaveAttribute("aria-invalid", "true");
    expect(field()).toHaveAccessibleDescription("There is no GitHub user named no-such-person.");
    expect(field()).toHaveFocus();
    expect(store.getState().view).toEqual({ kind: "mine" });
    // Typing again clears the message.
    await user.type(field(), "x");
    expect(within(dialog()).queryByText(/There is no GitHub user/)).toBeNull();
  });

  it("asks for a name when the field is empty", async () => {
    const user = userEvent.setup();
    await render();
    await openDialog(user);
    await user.keyboard("{Enter}");
    expect(within(dialog()).getByRole("alert")).toHaveTextContent("Type a GitHub username, or choose someone below.");
  });

  it("offers the people in the projects, most pull requests first, without those tracked", async () => {
    const user = userEvent.setup();
    const { store } = await render(withContributors(["kestrel-dev"]));
    await openDialog(user);
    const list = within(dialog()).getByRole("region", { name: "People with open pull requests in your projects" });
    const people = await within(list).findAllByRole("button", { name: /^Track / });
    expect(people.map((b) => b.getAttribute("aria-label"))).toEqual([
      "Track OrbitWren, 3 open pull requests",
      "Track tidepools, 2 open pull requests",
      "Track LumenAsh, 1 open pull request",
      "Track northwind-automation-collective, 1 open pull request",
    ]);
    await user.click(people[1]);
    await waitFor(() => expect(store.getState().view).toEqual({ kind: "contributor", login: "tidepools" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("narrows the list as you type, and Enter still tracks what was typed", async () => {
    const user = userEvent.setup();
    const { store } = await render();
    await openDialog(user);
    await within(dialog()).findByRole("button", { name: /^Track OrbitWren/ });
    await user.type(field(), "@lumen");
    const people = within(dialog()).getAllByRole("button", { name: /^Track [^,]+,/ });
    expect(people.map((b) => b.getAttribute("aria-label"))).toEqual(["Track LumenAsh, 1 open pull request"]);
    await user.type(field(), "x");
    expect(within(dialog()).getByText("Nobody here matches “lumenx”")).toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(await within(dialog()).findByRole("alert")).toHaveTextContent("There is no GitHub user named lumenx.");
    expect(store.getState().config?.contributors).toEqual([]);
  });

  it("shows avatars when there are some", async () => {
    const user = userEvent.setup();
    const known: KnownContributor[] = [{ login: "OrbitWren", avatarUrl: "https://example.test/a.png", openPrs: 3 }];
    await render(makeApi({}, { knownContributors: () => Promise.resolve(known) }));
    await openDialog(user);
    const row = await within(dialog()).findByRole("button", { name: "Track OrbitWren, 3 open pull requests" });
    expect(row.querySelector("img")).toHaveAttribute("src", "https://example.test/a.png");
  });

  it("shows a quiet loading state, then the list", async () => {
    const user = userEvent.setup();
    let resolve: (people: KnownContributor[]) => void = () => undefined;
    const pending = new Promise<KnownContributor[]>((r) => (resolve = r));
    await render(makeApi({}, { knownContributors: () => pending }));
    await openDialog(user);
    const list = within(dialog()).getByRole("region", { name: "People with open pull requests in your projects" });
    expect(list).toHaveAttribute("aria-busy", "true");
    expect(list).toHaveTextContent("Looking for people…");
    await act(async () => resolve([{ login: "LumenAsh", avatarUrl: null, openPrs: 1 }]));
    expect(list).toHaveAttribute("aria-busy", "false");
    expect(within(list).getByRole("button", { name: "Track LumenAsh, 1 open pull request" })).toBeInTheDocument();
  });

  it("says when nobody else has open pull requests", async () => {
    const user = userEvent.setup();
    await render(makeApi({ knownContributorsEmpty: true }));
    await openDialog(user);
    expect(await within(dialog()).findByText("Nobody else has open pull requests in your projects")).toBeInTheDocument();
  });

  it("says when the list could not load, and tries again; typing still works", async () => {
    const user = userEvent.setup();
    let fail = true;
    const base = makeApi();
    const api = makeApi(
      {},
      {
        knownContributors: () => (fail ? Promise.reject("GitHub did not answer.") : base.knownContributors()),
      },
    );
    const { store } = await render(api);
    await openDialog(user);
    expect(await within(dialog()).findByRole("alert")).toHaveTextContent(
      "Could not load the people in your projects: GitHub did not answer.",
    );
    fail = false;
    await user.click(within(dialog()).getByRole("button", { name: "Try again" }));
    expect(await within(dialog()).findByRole("button", { name: /^Track OrbitWren/ })).toBeInTheDocument();
    await user.type(field(), "tidepools{Enter}");
    await waitFor(() => expect(store.getState().view).toEqual({ kind: "contributor", login: "tidepools" }));
  });

  it("uses the mock option for a failing list", async () => {
    const user = userEvent.setup();
    await render(makeApi({ knownContributorsError: "Rate limit reached." }));
    await openDialog(user);
    expect(await within(dialog()).findByRole("alert")).toHaveTextContent(
      "Could not load the people in your projects: Rate limit reached.",
    );
  });

  it("closes on Esc and Cancel, back on the switcher", async () => {
    const user = userEvent.setup();
    await render();
    await openDialog(user);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(switcher()).toHaveFocus();
    await openDialog(user);
    await user.click(within(dialog()).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(switcher()).toHaveFocus();
  });
});

describe("mock contributors", () => {
  it("adds and removes a person's pull requests, keeping a followed one", async () => {
    const api = makeApi();
    const snapshots: number[] = [];
    await api.onSnapshot((s) => snapshots.push(s.projects.flatMap((p) => p.prs).length));
    const before = (await api.getSnapshot())!.projects.flatMap((p) => p.prs).length;

    await api.trackContributor("ORBITWREN");
    expect(snapshots).toEqual([before + 3]);
    // Tracking again changes nothing.
    expect((await api.trackContributor("@orbitwren")).contributors).toEqual(["OrbitWren"]);

    await api.followPr("dashpay/platform#6041");
    await api.untrackContributor("orbitwren");
    const after = (await api.getSnapshot())!.projects.flatMap((p) => p.prs);
    expect(after.length).toBe(before + 1);
    expect(after.find((p) => p.id === "dashpay/platform#6041")).toMatchObject({ isFollowed: true, isMine: false });
    expect((await api.getConfig()).contributors).toEqual([]);
  });

  it("refuses the viewer and unknown names", async () => {
    const api = makeApi();
    await expect(api.trackContributor("QuantumExplorer")).rejects.toThrow("That is you.");
    await expect(api.trackContributor("@ghost-user")).rejects.toThrow("There is no GitHub user named ghost-user.");
    await expect(api.trackContributor("  ")).rejects.toThrow("Type a GitHub username.");
  });

  it("keeps a tracked person's pull request when it is unfollowed", async () => {
    const api = makeApi();
    await api.trackContributor("tidepools");
    await api.followPr("dashpay/dips#197");
    await api.unfollowPr("dashpay/dips#197");
    const pr = (await api.getSnapshot())!.projects.flatMap((p) => p.prs).find((p) => p.id === "dashpay/dips#197");
    expect(pr).toMatchObject({ isFollowed: false });
  });
});
