export interface Entity {
  id: string;
}

function sealed(constructor: Function): void {
  Object.seal(constructor);
}

@sealed
export class Repo<T extends Entity> {
  private items = new Map<string, T>();

  add(item: T): void {
    this.items.set(item.id, item);
  }

  get(id: string): T | undefined {
    return this.items.get(id);
  }

  all(): T[] {
    return Array.from(this.items.values());
  }
}

export namespace Storage {
  export function key(id: string): string {
    return `repo:${id}`;
  }
}
