import { describe, expect, it } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PICKED_FOLDER } from "../../mock/run";
import { flush, renderApp } from "./testUtils";

type User = ReturnType<typeof userEvent.setup>;

async function openClone(user: User, repo: string) {
  await user.click(screen.getByRole("button", { name: `Actions for ${repo}` }));
  await user.click(screen.getByRole("menuitem", { name: "Local clone…" }));
  await flush();
  return screen.getByRole("dialog", { name: "Local clone" });
}

describe("the local clone dialog", () => {
  it("shows a clone found automatically and its example apps", async () => {
    const user = userEvent.setup();
    await renderApp();
    const dialog = await openClone(user, "dashpay/platform");
    expect(dialog).toHaveAccessibleDescription("dashpay/platform");
    expect(within(dialog).getByText("/Users/you/src/platform")).toBeInTheDocument();
    expect(within(dialog).getByText("Found automatically")).toBeInTheDocument();
    expect(within(dialog).getByText("iOS and Android")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "Find automatically" })).toBeNull();
  });

  it("says when no clone was found", async () => {
    const user = userEvent.setup();
    await renderApp();
    const dialog = await openClone(user, "dashpay/dips");
    expect(within(dialog).getByText("No local clone found")).toBeInTheDocument();
    expect(within(dialog).queryByText("Found automatically")).toBeNull();
  });

  it("sets a chosen folder, then goes back to finding it automatically", async () => {
    const user = userEvent.setup();
    const { store } = await renderApp();
    const dialog = await openClone(user, "dashpay/platform");
    await user.click(within(dialog).getByRole("button", { name: "Choose folder…" }));
    await flush();
    expect(within(dialog).getByText(PICKED_FOLDER)).toBeInTheDocument();
    expect(within(dialog).getByText("Set by you")).toBeInTheDocument();
    expect(store.getState().runTargets.find((t) => t.repo === "dashpay/platform")?.cloneIsSet).toBe(true);

    await user.click(within(dialog).getByRole("button", { name: "Find automatically" }));
    await flush();
    expect(within(dialog).getByText("Found automatically")).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "Find automatically" })).toBeNull();
    expect(within(dialog).getByRole("button", { name: "Choose folder…" })).toHaveFocus();
  });

  it("shows why a folder was refused, inline", async () => {
    const user = userEvent.setup();
    await renderApp({}, { pickFolder: async () => "/Users/you/src/nope" });
    const dialog = await openClone(user, "dashpay/platform");
    await user.click(within(dialog).getByRole("button", { name: "Choose folder…" }));
    await flush();
    expect(within(dialog).getByRole("alert")).toHaveTextContent("/Users/you/src/nope is not a clone of dashpay/platform.");
    expect(within(dialog).getByText("Found automatically")).toBeInTheDocument();
  });

  it("does nothing when the folder picker is cancelled", async () => {
    const user = userEvent.setup();
    await renderApp({}, { pickFolder: async () => null });
    const dialog = await openClone(user, "dashpay/platform");
    await user.click(within(dialog).getByRole("button", { name: "Choose folder…" }));
    await flush();
    expect(within(dialog).queryByRole("alert")).toBeNull();
    expect(within(dialog).getByText("/Users/you/src/platform")).toBeInTheDocument();
  });

  it("returns focus to the project's menu button when closed", async () => {
    const user = userEvent.setup();
    await renderApp();
    const dialog = await openClone(user, "dashpay/platform");
    await user.click(within(dialog).getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: "Actions for dashpay/platform" })).toHaveFocus();
  });
});
