import { useLayoutEffect, useRef, useState, type SelectHTMLAttributes } from "react";
import { Listbox, ListboxButton, ListboxOption, ListboxOptions } from "@headlessui/react";
import { Check, ChevronDown } from "lucide-react";
import { ScrollArea } from "./ScrollArea";

type Option = { value: string; label: string; disabled: boolean; hidden: boolean; title: string };
type Snapshot = { value: string; disabled: boolean; options: Option[] };

/** Keep the native value contract used by the client, without showing its picker. */
export function Select({ id, children, ...props }: SelectHTMLAttributes<HTMLSelectElement> & { id: string }) {
  const native = useRef<HTMLSelectElement>(null);
  const sync = useRef(() => {});
  const [snapshot, setSnapshot] = useState<Snapshot>({ value: "", disabled: !!props.disabled, options: [] });
  useLayoutEffect(() => {
    const node = native.current!;
    const read = () => {
      const options = Array.from(node.options, option => ({ value: option.value, label: option.label, disabled: option.disabled, hidden: Boolean(option.hidden), title: option.title }));
      setSnapshot(current => current.value === node.value && current.disabled === node.disabled && current.options.length === options.length && options.every((option, index) => (Object.keys(option) as (keyof Option)[]).every(key => current.options[index][key] === option[key]))
        ? current : { value: node.value, disabled: node.disabled, options });
    };
    sync.current = read;
    const observer = new MutationObserver(read);
    observer.observe(node, { subtree: true, childList: true, characterData: true, attributes: true });
    node.addEventListener("change", read);
    read();
    return () => { sync.current = () => {}; observer.disconnect(); node.removeEventListener("change", read); };
  }, []);
  // The client also assigns values directly before publishing its control snapshot.
  useLayoutEffect(() => { sync.current(); });
  const selected = snapshot.options.find(option => option.value === snapshot.value);
  return <div className="custom-select">
    <select {...props} id={id} ref={native} hidden aria-hidden="true" aria-label={undefined} aria-labelledby={undefined} tabIndex={-1}>{children}</select>
    <Listbox value={snapshot.value} disabled={props.disabled ?? snapshot.disabled} onChange={(value: string) => {
      const node = native.current!;
      if (node.disabled) return;
      node.value = value;
      node.dispatchEvent(new Event("change", { bubbles: true }));
      sync.current();
    }}>
      <ListboxButton id={`${id}-trigger`} className="custom-select-trigger" aria-label={props["aria-label"]} title={props.title}>
        <span className="custom-select-value">{selected?.label ?? ""}</span><ChevronDown size={16} aria-hidden="true" />
      </ListboxButton>
      <ListboxOptions anchor={{ to: "bottom start", gap: 6, padding: 12 }} portal modal={false} transition className="custom-select-options">
        <ScrollArea className="select-scroll-area" contentClassName="custom-select-list">
          {snapshot.options.filter(option => !option.hidden).map((option, index) => <ListboxOption key={`${option.value}:${index}`} value={option.value} disabled={option.disabled} title={option.title} className="custom-select-option">
            <Check size={15} className="custom-select-check" aria-hidden="true" /><span>{option.label}</span>
          </ListboxOption>)}
        </ScrollArea>
      </ListboxOptions>
    </Listbox>
  </div>;
}
