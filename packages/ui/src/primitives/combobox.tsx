import * as React from "react";
import { ChevronDownIcon } from "lucide-react";
import { cn } from "../lib/utils";
import { Input } from "./input";
import { Popover, PopoverAnchor, PopoverContent } from "./popover";

export interface ComboboxProps {
  value: string;
  onValueChange: (value: string) => void;
  options: ReadonlyArray<string>;
  placeholder?: string;
  disabled?: boolean;
  id?: string;
  className?: string;
  "aria-describedby"?: string;
  "aria-invalid"?: boolean;
  emptyLabel?: string;
  onKeyDown?: React.KeyboardEventHandler<HTMLInputElement>;
}

/** Free-text input with a list of suggestions (e.g. model ids). Width follows its container. */
export function Combobox({ value, onValueChange, options, placeholder, disabled, id, className, emptyLabel, onKeyDown, ...aria }: ComboboxProps) {
  const [open, setOpen] = React.useState(false);
  const q = value.trim().toLowerCase();
  const filtered = options.filter((o) => !q || o.toLowerCase().includes(q));
  const shown = filtered.length > 0 ? filtered : options;
  return (
    <Popover open={open && options.length > 0} onOpenChange={setOpen}>
      <PopoverAnchor asChild>
        <div className={cn("relative min-w-0", className)}>
          <Input
            id={id}
            value={value}
            disabled={disabled}
            placeholder={placeholder}
            className="pr-8 font-mono"
            autoComplete="off"
            role="combobox"
            aria-expanded={open}
            aria-autocomplete="list"
            onChange={(e) => onValueChange(e.target.value)}
            onFocus={() => setOpen(true)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setOpen(false);
              if (e.key === "ArrowDown") setOpen(true);
              onKeyDown?.(e);
            }}
            {...aria}
          />
          <button
            type="button"
            tabIndex={-1}
            aria-hidden
            disabled={disabled}
            onClick={() => setOpen((o) => !o)}
            className="absolute inset-y-0 right-0 flex w-8 items-center justify-center text-muted-foreground"
          >
            <ChevronDownIcon className="size-4" />
          </button>
        </div>
      </PopoverAnchor>
      <PopoverContent
        className="max-h-64 w-[var(--radix-popover-trigger-width)] overflow-auto"
        onOpenAutoFocus={(e) => e.preventDefault()}
        onInteractOutside={(e) => {
          if ((e.target as HTMLElement).closest("[data-slot=input]")) e.preventDefault();
        }}
      >
        <ul role="listbox">
          {shown.map((o) => (
            <li key={o} role="option" aria-selected={o === value}>
              <button
                type="button"
                className={cn("flex w-full rounded-md px-2 py-1.5 text-left font-mono text-xs hover:bg-surface-hover", o === value && "bg-surface-selected")}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => {
                  onValueChange(o);
                  setOpen(false);
                }}
              >
                {o}
              </button>
            </li>
          ))}
        </ul>
        {shown.length === 0 && emptyLabel ? <p className="px-2 py-1.5 text-xs text-muted-foreground">{emptyLabel}</p> : null}
      </PopoverContent>
    </Popover>
  );
}
