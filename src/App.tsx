// Composition root. The shell session owns this file.
import { Shell } from "./components/shell/Shell";
import { PrTable } from "./components/table/PrTable";
import { CommandPalette } from "./components/palette/CommandPalette";

export default function App() {
  return (
    <>
      <Shell>
        <PrTable />
      </Shell>
      <CommandPalette />
    </>
  );
}
