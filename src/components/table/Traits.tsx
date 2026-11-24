// The row's traits cell: one icon per trait (GroveDB change, contract change).
// An icon with one link opens it; with several it opens a menu of them; with
// none it is a plain picture with its summary as the tooltip.

import { useRef, useState, type MouseEvent } from "react";
import type { PrTrait, TraitKind } from "../../types";
import { useApp } from "../../state/store";
import { traitDescription, traitLabel, traitLinks } from "../../lib/traits";
import { Menu, MenuItem } from "../ui/Menu";
import "./Traits.css";

/**
 * A tree: one rounded, bumpy canopy on a trunk standing on the ground. GroveDB
 * is a tree of trees, and its workflow comment uses a tree for it. The ground
 * line and the single canopy keep it from reading as a club (♣).
 */
function TreeIcon() {
  return (
    <svg className="trait__icon" viewBox="0 0 14 14" aria-hidden="true" focusable="false">
      <g fill="currentColor">
        <circle cx="7" cy="3.9" r="3" />
        <circle cx="4.2" cy="5.6" r="2.5" />
        <circle cx="9.8" cy="5.6" r="2.5" />
        <circle cx="5.3" cy="7.4" r="2.2" />
        <circle cx="8.7" cy="7.4" r="2.2" />
        <rect x="6.25" y="7.5" width="1.5" height="5" />
        <rect x="2.6" y="12.1" width="8.8" height="1.2" rx="0.6" />
      </g>
    </svg>
  );
}

/** A page with a folded corner and three lines of text cut out: a contract's schema. */
function DocumentIcon() {
  return (
    <svg className="trait__icon" viewBox="0 0 14 14" aria-hidden="true" focusable="false">
      <g fill="currentColor">
        <path
          fillRule="evenodd"
          d="M3.2 1h4.9v3.1c0 .55.45 1 1 1h2.7v6.9c0 .55-.45 1-1 1H3.2c-.55 0-1-.45-1-1V2c0-.55.45-1 1-1ZM4.2 6.4h3.4v1.1H4.2ZM4.2 8.4h5.6v1.1H4.2ZM4.2 10.4h5.6v1.1H4.2Z"
        />
        <path d="M9.1 1.25 11.55 3.7c.16.16.05.4-.17.4H9.35a.25.25 0 0 1-.25-.25Z" />
      </g>
    </svg>
  );
}

export function TraitIcon({ kind }: { kind: TraitKind }) {
  return kind === "grovedbChange" ? <TreeIcon /> : <DocumentIcon />;
}

interface TraitButtonsProps {
  traits: PrTrait[];
  /** Whether the icons are in the Tab order: only in the row that has the list's Tab stop. */
  tabbable: boolean;
}

/** The icons, in the order given. */
export function TraitButtons({ traits, tabbable }: TraitButtonsProps) {
  return (
    <>
      {traits.map((t) => (
        <TraitButton key={t.kind} trait={t} tabbable={tabbable} />
      ))}
    </>
  );
}

function TraitButton({ trait, tabbable }: { trait: PrTrait; tabbable: boolean }) {
  const openUrl = useApp((s) => s.openUrl);
  const [menuOpen, setMenuOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const links = traitLinks(trait);
  const label = traitLabel(trait);

  if (links.length === 0) {
    return (
      <span className="trait inert" role="img" aria-label={label} title={label} data-trait={trait.kind}>
        <TraitIcon kind={trait.kind} />
      </span>
    );
  }

  const onClick = (e: MouseEvent) => {
    // The row behind the icon opens its detail on click; this click is the icon's.
    e.stopPropagation();
    if (links.length === 1) void openUrl(links[0].url);
    else setMenuOpen((o) => !o);
  };

  return (
    <>
      <button
        ref={button}
        type="button"
        className="trait"
        aria-label={label}
        title={label}
        aria-haspopup={links.length > 1 ? "menu" : undefined}
        aria-expanded={links.length > 1 ? menuOpen : undefined}
        tabIndex={tabbable ? 0 : -1}
        data-trait={trait.kind}
        onClick={onClick}
      >
        <TraitIcon kind={trait.kind} />
      </button>
      {menuOpen && (
        <Menu
          anchor={button}
          label={traitDescription(trait)}
          caption={traitDescription(trait)}
          align="start"
          onClose={(refocus) => {
            setMenuOpen(false);
            if (refocus) button.current?.focus();
          }}
        >
          {links.map((l, i) => (
            <MenuItem
              key={`${i}:${l.url}`}
              title={l.label}
              onSelect={() => {
                setMenuOpen(false);
                button.current?.focus();
                void openUrl(l.url);
              }}
            >
              {l.label}
            </MenuItem>
          ))}
        </Menu>
      )}
    </>
  );
}
