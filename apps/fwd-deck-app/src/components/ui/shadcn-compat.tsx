import * as CheckboxPrimitive from "@radix-ui/react-checkbox";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import * as LabelPrimitive from "@radix-ui/react-label";
import * as ProgressPrimitive from "@radix-ui/react-progress";
import * as RadixSwitchPrimitive from "@radix-ui/react-switch";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";

import { cn } from "../../lib/utils";

type ButtonVariant =
  | "danger"
  | "danger-soft"
  | "ghost"
  | "outline"
  | "primary"
  | "secondary"
  | "tertiary";

type ButtonSize = "lg" | "md" | "sm";

type AlertStatus = "accent" | "danger" | "success" | "warning";

type ChipColor = "accent" | "danger" | "default" | "success" | "warning";

type ChipVariant = "primary" | "secondary" | "soft" | "tertiary";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-[background-color,border-color,color,box-shadow] focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/20 disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        danger: "bg-destructive text-destructive-foreground shadow-sm hover:bg-destructive/90",
        "danger-soft": "text-destructive hover:bg-destructive/10 hover:text-destructive",
        ghost: "text-foreground/80 hover:bg-accent hover:text-foreground",
        outline:
          "border border-input bg-card shadow-sm hover:bg-accent hover:text-accent-foreground",
        primary: "bg-primary text-primary-foreground shadow-sm hover:bg-primary/90",
        secondary:
          "border border-transparent bg-secondary text-secondary-foreground shadow-sm hover:bg-secondary/80",
        tertiary: "bg-muted text-muted-foreground hover:bg-muted/80 hover:text-foreground",
      },
      size: {
        lg: "h-10 px-5",
        md: "h-9 px-4",
        sm: "h-8 px-3 text-xs",
      },
      fullWidth: {
        true: "w-full",
      },
      isIconOnly: {
        true: "size-8 p-0",
      },
    },
    defaultVariants: {
      variant: "primary",
      size: "md",
    },
  },
);

export interface ButtonProps
  extends
    Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "disabled">,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
  fullWidth?: boolean;
  isDisabled?: boolean;
  isIconOnly?: boolean;
  onPress?: () => void;
  size?: ButtonSize;
  slot?: string;
  variant?: ButtonVariant;
}

/**
 * shadcn/ui の button 仕様に寄せた操作ボタンを表示する
 */
export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    asChild = false,
    className,
    fullWidth,
    isDisabled,
    isIconOnly,
    onClick,
    onPress,
    size,
    slot,
    type = "button",
    variant,
    ...props
  },
  ref,
): React.ReactElement {
  const Comp = asChild ? Slot : "button";
  const handleClick = (event: React.MouseEvent<HTMLButtonElement>): void => {
    onClick?.(event);

    if (!event.defaultPrevented) {
      onPress?.();
    }
  };
  const button = (
    <Comp
      ref={ref}
      {...props}
      className={cn(buttonVariants({ fullWidth, isIconOnly, size, variant }), className)}
      data-slot={slot}
      disabled={isDisabled}
      onClick={handleClick}
      type={type}
    />
  );

  if (slot === "close") {
    return <DialogPrimitive.Close asChild>{button}</DialogPrimitive.Close>;
  }

  return button;
});
Button.displayName = "Button";

type CardProps = React.HTMLAttributes<HTMLDivElement> & {
  variant?: "default" | "secondary";
};

/**
 * shadcn/ui の card に相当する面を表示する
 */
export function Card({ className, variant = "default", ...props }: CardProps): React.ReactElement {
  return (
    <div
      className={cn(
        "rounded-xl border border-border bg-card text-card-foreground shadow-sm",
        variant === "secondary" && "bg-card",
        className,
      )}
      {...props}
    />
  );
}

interface ChipProps extends React.HTMLAttributes<HTMLSpanElement> {
  color?: ChipColor;
  size?: "md" | "sm";
  variant?: ChipVariant;
}

/**
 * shadcn/ui の badge に相当するチップを表示する
 */
export function Chip({
  className,
  color = "default",
  size = "md",
  variant = "secondary",
  ...props
}: ChipProps): React.ReactElement {
  return (
    <span
      className={cn(
        "inline-flex max-w-full items-center rounded-full border font-medium",
        size === "sm" ? "px-1.5 py-0.5 text-xs" : "px-2 py-0.5 text-sm",
        chipColorClassName(color, variant),
        className,
      )}
      {...props}
    />
  );
}

/**
 * チップの状態色をクラス名へ変換する
 */
function chipColorClassName(color: ChipColor, variant: ChipVariant): string {
  if (variant === "primary") {
    return color === "danger"
      ? "border-transparent bg-destructive text-destructive-foreground"
      : "border-transparent bg-primary text-primary-foreground";
  }

  if (color === "success") {
    return "border-transparent bg-emerald-500/10 text-emerald-700";
  }

  if (color === "warning") {
    return "border-transparent bg-amber-500/10 text-amber-700";
  }

  if (color === "danger") {
    return "border-transparent bg-destructive/10 text-destructive";
  }

  if (color === "accent") {
    return "border-transparent bg-primary/10 text-foreground";
  }

  return "border-transparent bg-muted text-muted-foreground";
}

interface AlertRootProps extends React.HTMLAttributes<HTMLDivElement> {
  status?: AlertStatus;
}

/**
 * shadcn/ui の alert に相当する通知面を表示する
 */
function AlertRoot({ className, status = "accent", ...props }: AlertRootProps): React.ReactElement {
  return (
    <div
      className={cn(
        "relative flex w-full items-start gap-3 rounded-lg border bg-card px-3.5 py-2.5 text-sm text-foreground shadow-sm",
        alertStatusClassName(status),
        className,
      )}
      {...props}
    />
  );
}

/**
 * 通知面の状態色をクラス名へ変換する
 */
function alertStatusClassName(status: AlertStatus): string {
  if (status === "danger") {
    return "border-destructive/20 border-l-4 border-l-destructive";
  }

  if (status === "success") {
    return "border-emerald-500/20 border-l-4 border-l-success";
  }

  if (status === "warning") {
    return "border-amber-500/25 border-l-4 border-l-warning";
  }

  return "border-border";
}

function AlertIndicator({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>): React.ReactElement {
  return <div className={cn("mt-0.5 shrink-0", className)} {...props} />;
}

function AlertContent({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>): React.ReactElement {
  return <div className={cn("min-w-0 flex-1", className)} {...props} />;
}

export const Alert = Object.assign(AlertRoot, {
  Content: AlertContent,
  Indicator: AlertIndicator,
});

interface TextFieldProps extends React.HTMLAttributes<HTMLDivElement> {
  isDisabled?: boolean;
  isRequired?: boolean;
  variant?: "primary" | "secondary";
}

/**
 * 入力欄とラベルを束ねる領域を表示する
 */
export function TextField({ className, ...props }: TextFieldProps): React.ReactElement {
  return <div className={cn("grid gap-1.5", className)} {...props} />;
}

type InputProps = React.InputHTMLAttributes<HTMLInputElement> & {
  fullWidth?: boolean;
  variant?: "primary" | "secondary";
};

/**
 * shadcn/ui の input に相当する入力欄を表示する
 */
export const Input = React.forwardRef<HTMLInputElement, InputProps>(function Input(
  { className, fullWidth, ...props },
  ref,
): React.ReactElement {
  return (
    <input
      ref={ref}
      className={cn(
        "flex h-9 rounded-md border border-input bg-card px-3 py-1 text-sm shadow-sm transition-[border-color,box-shadow] file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-50",
        fullWidth && "w-full",
        className,
      )}
      {...props}
    />
  );
});
Input.displayName = "Input";

/**
 * shadcn/ui の label に相当するラベルを表示する
 */
export const Label = React.forwardRef<
  React.ElementRef<typeof LabelPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof LabelPrimitive.Root>
>(({ className, ...props }, ref) => (
  <LabelPrimitive.Root
    ref={ref}
    className={cn(
      "text-sm leading-none font-medium peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
      className,
    )}
    {...props}
  />
));
Label.displayName = LabelPrimitive.Root.displayName;

type CheckboxSelectionState = boolean | "indeterminate";

interface CheckboxContextValue {
  disabled: boolean;
  onChange?: (checked: boolean) => void;
  selected: CheckboxSelectionState;
}

const CheckboxContext = React.createContext<CheckboxContextValue | null>(null);

interface CheckboxRootProps extends Omit<React.LabelHTMLAttributes<HTMLLabelElement>, "onChange"> {
  isDisabled?: boolean;
  isSelected?: CheckboxSelectionState;
  onChange?: (checked: boolean) => void;
}

/**
 * shadcn/ui の checkbox に相当する選択部品を表示する
 */
function CheckboxRoot({
  children,
  className,
  isDisabled = false,
  isSelected = false,
  onChange,
  ...props
}: CheckboxRootProps): React.ReactElement {
  return (
    <CheckboxContext.Provider value={{ disabled: isDisabled, onChange, selected: isSelected }}>
      <label className={cn("inline-flex items-center gap-2 text-sm", className)} {...props}>
        {children}
      </label>
    </CheckboxContext.Provider>
  );
}

function CheckboxControl({
  children,
  className,
}: React.HTMLAttributes<HTMLButtonElement>): React.ReactElement {
  const context = useRequiredContext(CheckboxContext, "Checkbox.Control");

  return (
    <CheckboxPrimitive.Root
      checked={context.selected}
      className={cn(
        "peer size-4 shrink-0 rounded-sm border border-input bg-card shadow-sm focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:border-primary data-[state=checked]:bg-primary data-[state=checked]:text-primary-foreground data-[state=indeterminate]:border-primary data-[state=indeterminate]:bg-primary data-[state=indeterminate]:text-primary-foreground",
        className,
      )}
      disabled={context.disabled}
      onCheckedChange={(checked) => context.onChange?.(checked === true)}
    >
      {children}
    </CheckboxPrimitive.Root>
  );
}

function CheckboxIndicator({
  children,
  className,
}: React.HTMLAttributes<HTMLSpanElement>): React.ReactElement {
  const context = useRequiredContext(CheckboxContext, "Checkbox.Indicator");

  return (
    <CheckboxPrimitive.Indicator
      className={cn("flex items-center justify-center text-current", className)}
    >
      {children ??
        (context.selected === "indeterminate" ? (
          <svg aria-hidden="true" className="size-3" fill="none" viewBox="0 0 16 16">
            <path d="M3.5 8H12.5" stroke="currentColor" strokeLinecap="round" strokeWidth="2" />
          </svg>
        ) : (
          <svg aria-hidden="true" className="size-3" fill="none" viewBox="0 0 16 16">
            <path
              d="M3.5 8.5 6.5 11.5 12.5 4.5"
              stroke="currentColor"
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth="2"
            />
          </svg>
        ))}
    </CheckboxPrimitive.Indicator>
  );
}

function CheckboxContent({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>): React.ReactElement {
  return <div className={cn("grid gap-0.5", className)} {...props} />;
}

export const Checkbox = Object.assign(CheckboxRoot, {
  Content: CheckboxContent,
  Control: CheckboxControl,
  Indicator: CheckboxIndicator,
});

interface SwitchContextValue {
  disabled: boolean;
  onChange?: (checked: boolean) => void;
  selected: boolean;
}

const SwitchContext = React.createContext<SwitchContextValue | null>(null);

interface SwitchRootProps extends Omit<React.HTMLAttributes<HTMLDivElement>, "onChange"> {
  isSelected?: boolean;
  isDisabled?: boolean;
  onChange?: (checked: boolean) => void;
  size?: "sm";
}

/**
 * shadcn/ui の switch に相当する切替部品を表示する
 */
function SwitchRoot({
  children,
  className,
  isDisabled = false,
  isSelected = false,
  onChange,
  ...props
}: SwitchRootProps): React.ReactElement {
  return (
    <SwitchContext.Provider value={{ disabled: isDisabled, onChange, selected: isSelected }}>
      <div className={cn("flex items-center gap-3", className)} {...props}>
        {children}
      </div>
    </SwitchContext.Provider>
  );
}

function SwitchContent({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>): React.ReactElement {
  return <div className={cn("min-w-0 flex-1", className)} {...props} />;
}

function SwitchControl({
  children,
  className,
  ...props
}: React.HTMLAttributes<HTMLButtonElement>): React.ReactElement {
  const context = useRequiredContext(SwitchContext, "Switch.Control");

  return (
    <RadixSwitchPrimitive.Root
      {...props}
      checked={context.selected}
      className={cn(
        "peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-muted",
        className,
      )}
      disabled={context.disabled}
      onCheckedChange={context.onChange}
    >
      {children}
    </RadixSwitchPrimitive.Root>
  );
}

function SwitchThumb({ className }: React.HTMLAttributes<HTMLSpanElement>): React.ReactElement {
  return (
    <RadixSwitchPrimitive.Thumb
      className={cn(
        "pointer-events-none block size-4 rounded-full bg-card shadow-md ring-0 transition-transform data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0",
        className,
      )}
    />
  );
}

export const Switch = Object.assign(SwitchRoot, {
  Content: SwitchContent,
  Control: SwitchControl,
  Thumb: SwitchThumb,
});

interface ModalRootProps {
  children: React.ReactNode;
  isOpen: boolean;
  onOpenChange?: (open: boolean) => void;
}

/**
 * shadcn/ui の dialog に相当するモーダルを表示する
 */
function ModalRoot({ children, isOpen, onOpenChange }: ModalRootProps): React.ReactElement {
  return (
    <DialogPrimitive.Root open={isOpen} onOpenChange={onOpenChange}>
      {children}
    </DialogPrimitive.Root>
  );
}

interface ModalBackdropProps {
  children: React.ReactNode;
  isDismissable?: boolean;
  variant?: "blur";
}

function ModalBackdrop({ children }: ModalBackdropProps): React.ReactElement {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/35 backdrop-blur-sm" />
      {children}
    </DialogPrimitive.Portal>
  );
}

interface ModalContainerProps extends React.HTMLAttributes<HTMLDivElement> {
  placement?: "center";
  scroll?: "inside";
  size?: "lg" | "sm";
}

function ModalContainer({ className, ...props }: ModalContainerProps): React.ReactElement {
  return (
    <div
      className={cn("fixed inset-0 z-50 flex items-center justify-center p-4", className)}
      {...props}
    />
  );
}

function ModalDialog({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>): React.ReactElement {
  return (
    <DialogPrimitive.Content
      className={cn(
        "max-h-[calc(100vh-2rem)] rounded-xl border border-border bg-card text-foreground shadow-2xl outline-none",
        className,
      )}
      {...props}
    />
  );
}

export const Modal = Object.assign(ModalRoot, {
  Backdrop: ModalBackdrop,
  Container: ModalContainer,
  Dialog: ModalDialog,
});

interface DropdownActionContextValue {
  onAction?: (key: React.Key) => void;
}

const DropdownActionContext = React.createContext<DropdownActionContextValue>({});

interface DropdownMenuProps extends React.ComponentPropsWithoutRef<
  typeof DropdownMenuPrimitive.Content
> {
  onAction?: (key: React.Key) => void;
}

function DropdownRoot(
  props: React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Root>,
): React.ReactElement {
  return <DropdownMenuPrimitive.Root {...props} />;
}

interface DropdownTriggerProps extends Omit<
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Trigger>,
  "disabled"
> {
  isDisabled?: boolean;
}

function DropdownTrigger({
  className,
  isDisabled,
  ...props
}: DropdownTriggerProps): React.ReactElement {
  return (
    <DropdownMenuPrimitive.Trigger
      className={cn(buttonVariants({ isIconOnly: true, size: "sm", variant: "ghost" }), className)}
      disabled={isDisabled}
      {...props}
    />
  );
}

function DropdownPopover({
  children,
  className,
  placement = "bottom",
  ...props
}: React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Content> & {
  placement?: "bottom end" | "bottom";
}): React.ReactElement {
  const [side, align] = placement.split(" ") as ["bottom", "end" | undefined];

  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        align={align}
        className={cn(
          "z-50 min-w-56 overflow-hidden rounded-lg border bg-popover p-1 text-popover-foreground shadow-lg",
          className,
        )}
        side={side}
        {...props}
      >
        {children}
      </DropdownMenuPrimitive.Content>
    </DropdownMenuPrimitive.Portal>
  );
}

function DropdownMenu({ children, onAction }: DropdownMenuProps): React.ReactElement {
  return (
    <DropdownActionContext.Provider value={{ onAction }}>{children}</DropdownActionContext.Provider>
  );
}

interface DropdownItemProps extends Omit<
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Item>,
  "disabled" | "id"
> {
  id: React.Key;
  isDisabled?: boolean;
  textValue?: string;
}

function DropdownItem({
  children,
  className,
  id,
  isDisabled,
  ...props
}: DropdownItemProps): React.ReactElement {
  const { onAction } = React.useContext(DropdownActionContext);

  return (
    <DropdownMenuPrimitive.Item
      className={cn(
        "relative flex cursor-default select-none items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-none transition-colors focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
        className,
      )}
      disabled={isDisabled}
      onSelect={() => onAction?.(id)}
      {...props}
    >
      {children}
    </DropdownMenuPrimitive.Item>
  );
}

export const Dropdown = Object.assign(DropdownRoot, {
  Item: DropdownItem,
  Menu: DropdownMenu,
  Popover: DropdownPopover,
  Trigger: DropdownTrigger,
});

interface TooltipRootProps {
  children: React.ReactNode;
}

/**
 * shadcn/ui の tooltip に相当する補助表示を提供する
 */
function TooltipRoot({ children }: TooltipRootProps): React.ReactElement {
  const childArray = React.Children.toArray(children);
  const [trigger, ...content] = childArray;

  return (
    <TooltipPrimitive.Provider>
      <TooltipPrimitive.Root>
        <TooltipPrimitive.Trigger asChild>{trigger as React.ReactElement}</TooltipPrimitive.Trigger>
        {content}
      </TooltipPrimitive.Root>
    </TooltipPrimitive.Provider>
  );
}

function TooltipContent({
  children,
  className,
  placement = "top",
  showArrow,
}: React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content> & {
  placement?: "left" | "top";
  showArrow?: boolean;
}): React.ReactElement {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        className={cn(
          "z-50 overflow-hidden rounded-md bg-foreground px-3 py-1.5 text-xs text-background shadow-md",
          className,
        )}
        side={placement}
        sideOffset={4}
      >
        {children}
        {showArrow ? <TooltipPrimitive.Arrow className="fill-foreground" /> : null}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}

export const Tooltip = Object.assign(TooltipRoot, {
  Content: TooltipContent,
});

interface ProgressBarContextValue {
  maxValue: number;
  value: number;
}

const ProgressBarContext = React.createContext<ProgressBarContextValue | null>(null);

interface ProgressBarRootProps extends React.HTMLAttributes<HTMLDivElement> {
  color?: ChipColor;
  maxValue?: number;
  size?: "sm";
  value: number;
}

/**
 * shadcn/ui の progress に相当する進捗表示を提供する
 */
function ProgressBarRoot({
  children,
  className,
  maxValue = 100,
  value,
  ...props
}: ProgressBarRootProps): React.ReactElement {
  return (
    <ProgressBarContext.Provider value={{ maxValue, value }}>
      <ProgressPrimitive.Root
        className={cn("relative h-2 w-full overflow-hidden rounded-full bg-muted", className)}
        max={maxValue}
        value={value}
        {...props}
      >
        {children}
      </ProgressPrimitive.Root>
    </ProgressBarContext.Provider>
  );
}

function ProgressBarTrack({ children }: React.HTMLAttributes<HTMLDivElement>): React.ReactElement {
  return <>{children}</>;
}

function ProgressBarFill({ className }: React.HTMLAttributes<HTMLDivElement>): React.ReactElement {
  const context = useRequiredContext(ProgressBarContext, "ProgressBar.Fill");
  const percentage = context.maxValue === 0 ? 0 : (context.value / context.maxValue) * 100;

  return (
    <ProgressPrimitive.Indicator
      className={cn("h-full w-full flex-1 bg-primary transition-all", className)}
      style={{ transform: `translateX(-${100 - percentage}%)` }}
    />
  );
}

export const ProgressBar = Object.assign(ProgressBarRoot, {
  Fill: ProgressBarFill,
  Track: ProgressBarTrack,
});

interface TableRootProps extends React.HTMLAttributes<HTMLDivElement> {
  variant?: "secondary";
}

function TableRoot({ className, variant, ...props }: TableRootProps): React.ReactElement {
  return (
    <div className={cn("w-full", variant === "secondary" && "rounded-md", className)} {...props} />
  );
}

function TableScrollContainer({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>): React.ReactElement {
  return <div className={cn("w-full overflow-auto", className)} {...props} />;
}

function TableContent({
  className,
  ...props
}: React.TableHTMLAttributes<HTMLTableElement>): React.ReactElement {
  return <table className={cn("w-full caption-bottom text-sm", className)} {...props} />;
}

function TableHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLTableSectionElement>): React.ReactElement {
  return <thead className={cn("bg-muted/45 [&_tr]:border-b", className)} {...props} />;
}

function TableBody(props: React.HTMLAttributes<HTMLTableSectionElement>): React.ReactElement {
  return <tbody className="[&_tr:last-child]:border-0" {...props} />;
}

interface TableColumnProps extends React.ThHTMLAttributes<HTMLTableCellElement> {
  isRowHeader?: boolean;
}

function TableColumn({
  className,
  isRowHeader,
  scope,
  ...props
}: TableColumnProps): React.ReactElement {
  return (
    <th
      className={cn(
        "h-9 px-3 text-left align-middle text-xs font-medium text-muted-foreground [&:has([role=checkbox])]:pr-0",
        className,
      )}
      scope={scope ?? (isRowHeader ? "col" : undefined)}
      {...props}
    />
  );
}

function TableRow({
  className,
  ...props
}: React.HTMLAttributes<HTMLTableRowElement>): React.ReactElement {
  return (
    <tr
      className={cn(
        "border-b transition-colors hover:bg-muted/40 data-[state=selected]:bg-muted",
        className,
      )}
      {...props}
    />
  );
}

function TableCell({
  className,
  ...props
}: React.TdHTMLAttributes<HTMLTableCellElement>): React.ReactElement {
  return (
    <td
      className={cn("px-3 py-2.5 align-middle [&:has([role=checkbox])]:pr-0", className)}
      {...props}
    />
  );
}

export const Table = Object.assign(TableRoot, {
  Body: TableBody,
  Cell: TableCell,
  Column: TableColumn,
  Content: TableContent,
  Header: TableHeader,
  Row: TableRow,
  ScrollContainer: TableScrollContainer,
});

/**
 * 必須コンテキストの未設定を実行時に検出する
 */
function useRequiredContext<T>(context: React.Context<T | null>, label: string): T {
  const value = React.useContext(context);
  if (value === null) {
    throw new Error(`${label} は対応する Root 内で使用してください`);
  }

  return value;
}
