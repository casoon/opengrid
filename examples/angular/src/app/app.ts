// opengrid in Angular (plan point 81): the element in the template, supplied
// by the directive from docs/guides/frameworks.md, its view two-way bound (or
// one-way with `?one-way`), and a switch that takes the grid out and puts it
// back.
import { CUSTOM_ELEMENTS_SCHEMA, Component, VERSION, inject } from "@angular/core";
import type { SelectionChangeDetail, Texts, View } from "@casoon/opengrid";
import { OpengridDirective } from "./opengrid.directive";
import { ORDERS } from "./provider";

@Component({
  selector: "app-root",
  imports: [OpengridDirective],
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
  template: `
    <main>
      <h1>opengrid in Angular {{ version }}</h1>
      <p>
        <button type="button" (click)="shown = !shown">
          {{ shown ? "Hide the grid" : "Show the grid" }}
        </button>
        <button type="button" (click)="view = saved">Restore the saved view</button>
        <button type="button" [attr.aria-pressed]="german" (click)="german = !german">German</button>
      </p>
      <p id="selected">{{ selected }} rows selected</p>
      @if (shown) {
        @if (oneWay) {
          <!-- ?one-way: the page sets the view; the reader's change stays. -->
          <opengrid-grid
          opengrid
            label="Orders"
            datasource="orders"
            columns="id,customer,country,amount,qty"
            window-size="40"
            selection
            toolbar
            class="orders"
            [provider]="provider"
            [texts]="german ? germanTexts : undefined"
            [view]="view"
            (opengrid-selection-change)="onSelection($event)"
          ></opengrid-grid>
        } @else {
          <opengrid-grid
          opengrid
            label="Orders"
            datasource="orders"
            columns="id,customer,country,amount,qty"
            window-size="40"
            selection
            toolbar
            class="orders"
            [provider]="provider"
            [texts]="german ? germanTexts : undefined"
            [(view)]="view"
            (opengrid-selection-change)="onSelection($event)"
          ></opengrid-grid>
        }
      }
    </main>
  `,
})
export class App {
  protected readonly version = VERSION.full;
  protected readonly provider = inject(ORDERS);
  protected readonly saved: View = {
    sort: [{ field: "customer", direction: "asc" }],
  } as View;
  protected readonly germanTexts: Texts = {
    lang: "de",
    matchesOne: "{count} Treffer",
    matchesOther: "{count} Treffer",
  };
  /** `?one-way`: `[view]` instead of `[(view)]` (plan point 81). */
  protected readonly oneWay = location.search.includes("one-way");
  protected shown = true;
  protected german = false;
  protected selected = 0;

  private currentView: View | null = { sort: [{ field: "id", direction: "asc" }] } as View;

  /** The view as Angular holds it; mirrored onto `window` for the tests. */
  protected get view(): View | null {
    return this.currentView;
  }
  protected set view(next: View | null) {
    this.currentView = next;
    (window as unknown as { __view: View | null }).__view = next;
  }

  protected onSelection(event: Event): void {
    this.selected = (event as CustomEvent<SelectionChangeDetail>).detail.count;
  }
}
