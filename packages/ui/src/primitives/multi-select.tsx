import { ChevronDownIcon } from "lucide-react";
import { cn } from "../lib/utils";
import { controlClasses } from "./input";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuTrigger } from "./dropdown-menu";

export interface MultiSelectProps {
  options: ReadonlyArray<{ value: string; label: string }>;
  selected: ReadonlyArray<string>;
  onChange: (selected: string[]) => void;
  /** Shown when nothing is selected. */
  placeholder: string;
  /** Text for the current selection; defaults to the labels joined by commas. */
  summary?: string;
  "aria-label"?: string;
  disabled?: boolean;
  className?: string;
}

/** Filter that allows several values: a menu of checkboxes that stays open while ticking. */
export function MultiSelect({ options, selected, onChange, placeholder, summary, disabled, className, ...aria }: MultiSelectProps) {
  const text = selected.length === 0 ? placeholder : (summary ?? options.filter((o) => selected.includes(o.value)).map((o) => o.label).join(", "));
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled}
        aria-label={aria["aria-label"]}
        className={cn(controlClasses, "flex h-8 items-center justify-between gap-2 text-left", selected.length === 0 && "text-subtle-foreground", className)}
      >
        <span className="truncate">{text}</span>
        <ChevronDownIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-72 min-w-[var(--radix-dropdown-menu-trigger-width)] overflow-auto">
        {options.map((option) => (
          <DropdownMenuCheckboxItem
            key={option.value}
            checked={selected.includes(option.value)}
            onSelect={(e) => e.preventDefault()}
            onCheckedChange={(checked) =>
              onChange(checked ? [...selected, option.value] : selected.filter((value) => value !== option.value))
            }
          >
            {option.label}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
