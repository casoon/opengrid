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
import { connect, type Connection, type ConnectOptions, type View } from "@casoon/opengrid";

@Directive({ selector: "[opengrid]" })
export class OpengridDirective implements OnChanges, OnDestroy {
  @Input() provider?: ConnectOptions["provider"];
  @Input() texts?: ConnectOptions["texts"];
  @Input() view?: ConnectOptions["view"];
  @Output() viewChange = new EventEmitter<View>();

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private connection?: Connection;

  constructor() {
    // In the browser only, once rendered — never during server rendering.
    afterNextRender(() => {
      this.connection = connect(this.host.nativeElement, {
        provider: this.provider,
        texts: this.texts,
        view: this.view,
        onViewChange: (view) => this.viewChange.emit(view),
      });
    });
  }

  // Only the inputs that changed: an unchanged `view` passed along with new
  // texts would put back a view the reader has changed since.
  ngOnChanges(changes: SimpleChanges): void {
    const changed: ConnectOptions = {};
    if ("provider" in changes) changed.provider = this.provider;
    if ("texts" in changes) changed.texts = this.texts;
    if ("view" in changes) changed.view = this.view;
    this.connection?.update(changed);
  }

  ngOnDestroy(): void {
    this.connection?.disconnect();
  }
}
