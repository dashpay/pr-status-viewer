import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { makeApi, renderShell } from "../shell/testUtils";

const sidebar = () => screen.getByRole("complementary", { name: "Views and projects" });

async function openAdd(user: ReturnType<typeof userEvent.setup>) {
  await user.click(within(sidebar()).getByRole("button", { name: "+ Add project" }));
  return screen.getByRole("dialog", { name: "Add project" });
}

async function openMenu(user: ReturnType<typeof userEvent.setup>, repo: string) {
  await user.click(within(sidebar()).getByRole("button", { name: `Actions for ${repo}` }));
  return screen.getByRole("menu", { name: `Actions for ${repo}` });
}

describe("add project", () => {
  it("adds a project from a GitHub link and selects it", async () => {
    const user = userEvent.setup();
    const { store } = await renderShell();
    const dialog = await openAdd(user);
    const field = within(dialog).getByRole("textbox", { name: "Repository" });
    expect(field).toHaveFocus();
    await user.type(field, "https://github.com/rust-lang/rust{Enter}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(store.getState().config?.projects).toContain("rust-lang/rust");
    expect(store.getState().view).toEqual({ kind: "project", repo: "rust-lang/rust" });
    expect(within(sidebar()).getByRole("button", { name: /^rust-lang\/rust/ })).toHaveAttribute("aria-current", "page");
  });

  it("keeps the dialog open and explains a duplicate", async () => {
    const user = userEvent.setup();
    await renderShell();
    const dialog = await openAdd(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Repository" }), "dashpay/dips{Enter}");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("dashpay/dips is already in your projects.");
    expect(screen.getByRole("dialog", { name: "Add project" })).toBeInTheDocument();
    expect(within(dialog).getByRole("textbox", { name: "Repository" })).toHaveAttribute("aria-invalid", "true");
  });

  it("explains malformed input", async () => {
    const user = userEvent.setup();
    await renderShell();
    const dialog = await openAdd(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Repository" }), "not a repo");
    await user.click(within(dialog).getByRole("button", { name: "Add project" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/owner\/name/);
  });

  it("shows progress while adding", async () => {
    const user = userEvent.setup();
    let resolve: () => void = () => {};
    const base = makeApi();
    const api = makeApi({}, {
      addProject: (input) => new Promise((r) => (resolve = () => r(base.addProject(input)))),
    });
    await renderShell(api);
    const dialog = await openAdd(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Repository" }), "a/b{Enter}");
    expect(within(dialog).getByRole("button", { name: "Adding…" })).toBeDisabled();
    resolve();
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("cancels with Esc and returns focus", async () => {
    const user = userEvent.setup();
    await renderShell();
    const add = within(sidebar()).getByRole("button", { name: "+ Add project" });
    await openAdd(user);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(add).toHaveFocus();
  });

  it("keeps Tab inside the dialog", async () => {
    const user = userEvent.setup();
    await renderShell();
    const dialog = await openAdd(user);
    await user.type(within(dialog).getByRole("textbox"), "x");
    for (let i = 0; i < 5; i++) {
      await user.tab();
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
  });
});

describe("remove project", () => {
  it("asks for confirmation naming the repo, then removes it", async () => {
    const user = userEvent.setup();
    const { store } = await renderShell();
    const menu = await openMenu(user, "dashpay/dips");
    await user.click(within(menu).getByRole("menuitem", { name: "Remove…" }));
    const dialog = screen.getByRole("dialog", { name: "Remove dashpay/dips?" });

    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(store.getState().config?.projects).toContain("dashpay/dips");

    await user.click(within(await openMenu(user, "dashpay/dips")).getByRole("menuitem", { name: "Remove…" }));
    await user.click(
      within(screen.getByRole("dialog", { name: "Remove dashpay/dips?" })).getByRole("button", { name: "Remove project" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(store.getState().config?.projects).not.toContain("dashpay/dips");
    expect(within(sidebar()).queryByRole("button", { name: /^dashpay\/dips/ })).not.toBeInTheDocument();
  });

  it("closes the menu with Esc", async () => {
    const user = userEvent.setup();
    await renderShell();
    await openMenu(user, "dashpay/dash");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(within(sidebar()).getByRole("button", { name: "Actions for dashpay/dash" })).toHaveFocus();
  });
});

describe("follow picker", () => {
  async function openPicker(user: ReturnType<typeof userEvent.setup>, repo = "dashpay/platform") {
    await user.click(within(await openMenu(user, repo)).getByRole("menuitem", { name: "Browse open PRs" }));
    const dialog = screen.getByRole("dialog", { name: `Follow pull requests in ${repo}` });
    await within(dialog).findByRole("list", { name: `Open pull requests in ${repo}` });
    return dialog;
  }

  it("lists open PRs and toggles following", async () => {
    const user = userEvent.setup();
    const { store } = await renderShell();
    const dialog = await openPicker(user);
    const toggle = within(dialog).getByRole("button", { name: /^Follow #9000 / });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    await user.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "true"));
    expect(toggle).toHaveTextContent("Following");
    expect(store.getState().config?.followed).toContain("dashpay/platform#9000");

    await user.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "false"));
    expect(store.getState().config?.followed).not.toContain("dashpay/platform#9000");
  });

  it("shows Yours instead of a toggle on the viewer's own PRs", async () => {
    const user = userEvent.setup();
    await renderShell();
    const dialog = await openPicker(user);
    const own = within(dialog).getByText(/#6033/).closest("li") as HTMLElement;
    expect(own).toHaveTextContent("Yours");
    expect(within(own).queryByRole("button")).not.toBeInTheDocument();
  });

  it("filters the list", async () => {
    const user = userEvent.setup();
    await renderShell();
    const dialog = await openPicker(user);
    await user.type(within(dialog).getByRole("searchbox", { name: "Filter open pull requests" }), "grovedb");
    const rows = within(within(dialog).getByRole("list")).getAllByRole("listitem");
    expect(rows).toHaveLength(1);
    expect(rows[0]).toHaveTextContent("batch grovedb operations");
    await user.clear(within(dialog).getByRole("searchbox"));
    await user.type(within(dialog).getByRole("searchbox"), "zzz-nothing");
    expect(within(dialog).getByText(/No open pull requests match/)).toBeInTheDocument();
  });

  it("follows a PR by link", async () => {
    const user = userEvent.setup();
    const { store } = await renderShell();
    const dialog = await openPicker(user);
    const field = within(dialog).getByRole("textbox", { name: "Follow by link" });
    expect(field).toHaveFocus();
    await user.type(field, "https://github.com/rust-lang/rust/pull/42{Enter}");
    expect(await within(dialog).findByText("Following https://github.com/rust-lang/rust/pull/42.")).toBeInTheDocument();
    expect(store.getState().config?.followed).toContain("rust-lang/rust#42");

    await user.type(field, "nonsense{Enter}");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/not a pull request/);
  });

  it("shows the empty state", async () => {
    const user = userEvent.setup();
    await renderShell(makeApi({}, { listOpenPrs: async () => [] }));
    await user.click(within(await openMenu(user, "dashpay/dips")).getByRole("menuitem", { name: "Browse open PRs" }));
    expect(await screen.findByText("No open pull requests")).toBeInTheDocument();
  });

  it("shows a load error with Try again", async () => {
    const user = userEvent.setup();
    let fail = true;
    const base = makeApi();
    await renderShell(
      makeApi({}, { listOpenPrs: (repo) => (fail ? Promise.reject("GitHub said no") : base.listOpenPrs(repo)) }),
    );
    await user.click(within(await openMenu(user, "dashpay/dips")).getByRole("menuitem", { name: "Browse open PRs" }));
    expect(await screen.findByText(/GitHub said no/)).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("list", { name: "Open pull requests in dashpay/dips" })).toBeInTheDocument();
  });

  it("closes on a backdrop click", async () => {
    const user = userEvent.setup();
    await renderShell();
    const dialog = await openPicker(user);
    await user.pointer({ keys: "[MouseLeft]", target: dialog.parentElement as HTMLElement });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
