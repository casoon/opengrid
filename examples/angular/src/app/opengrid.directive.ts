import {
  Directive,
  ElementRef,
  EventEmitter,
  Input,
  OnChanges,
  OnDestroy,
  Output,
  SimpleChanges,
  afterNextRender,
  inject,
} from "@angular/core";
import {
  connect,
  type CellWrites,
  type Connection,
  type ConnectOptions,
  type View,
} from "@casoon/opengrid";

@Directive({ selector: "[opengrid]", exportAs: "opengrid" })
export class OpengridDirective implements OnChanges, OnDestroy, CellWrites {
  @Input() provider?: ConnectOptions["provider"];
  @Input() texts?: ConnectOptions["texts"];
  @Input() view?: ConnectOptions["view"];
  @Output() viewChange = new EventEmitter<View>();
  // Editing (the grid only). The answer to a new row is the function's return
  // value, which an @Output cannot give.
  @Input() rowKey?: string;
  @Input() addRow?: boolean;
  @Input() actions?: ConnectOptions["actions"];
  @Input() validators?: ConnectOptions["validators"];
  @Input() rowAdd?: ConnectOptions["onRowAdd"];

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private connection?: Connection;

  constructor() {
    // In the browser only, once rendered — never during server rendering.
    afterNextRender(() => {
      this.connection = connect(this.host.nativeElement, {
        provider: this.provider,
        texts: this.texts,
        view: this.view,
        actions: this.actions,
        validators: this.validators,
        onViewChange: (view) => this.viewChange.emit(view),
        onRowAdd: (detail) => this.rowAdd?.(detail),
      });
    });
  }

  // Only the inputs that changed: an unchanged `view` passed along with new
  // texts would put back a view the reader has changed since.
  ngOnChanges(changes: SimpleChanges): void {
    const element = this.host.nativeElement;
    if ("rowKey" in changes) {
      if (this.rowKey) element.setAttribute("row-key", this.rowKey);
      else element.removeAttribute("row-key");
    }
    if ("addRow" in changes) {
      if (this.addRow) element.setAttribute("add-row", "");
      else element.removeAttribute("add-row");
    }
    const changed: ConnectOptions = {};
    if ("provider" in changes) changed.provider = this.provider;
    if ("texts" in changes) changed.texts = this.texts;
    if ("view" in changes) changed.view = this.view;
    if ("actions" in changes) changed.actions = this.actions;
    if ("validators" in changes) changed.validators = this.validators;
    this.connection?.update(changed);
  }

  /** What became of an edit the page saves — `#grid="opengrid"` in the template. */
  setCellState(...args: Parameters<CellWrites["setCellState"]>): void {
    this.connection?.setCellState(...args);
  }

  setValues(values: Parameters<CellWrites["setValues"]>[0]): void {
    this.connection?.setValues(values);
  }

  ngOnDestroy(): void {
    this.connection?.disconnect();
  }
}
