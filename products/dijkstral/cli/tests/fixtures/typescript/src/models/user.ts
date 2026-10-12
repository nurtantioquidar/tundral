import type { Entity } from "./repo";

/** Role of a user in the system. */
export enum Role {
  Member = "member",
  Admin = "admin",
}

export type UserId = string;

export interface Named {
  name: string;
}

export abstract class Base implements Entity {
  id: UserId = "";

  abstract describe(): string;

  save(): boolean {
    // NOTE: persistence is faked in this fixture.
    return validate(this);
  }
}

export class User extends Base implements Named {
  static count = 0;

  constructor(public name: string, private role: Role = Role.Member) {
    super();
    User.count += 1;
  }

  get display(): string {
    return formatName(this.name);
  }

  static parse(raw: string): User {
    return new User(raw.trim());
  }

  describe(): string {
    return `${this.name} (${this.role})`;
  }
}

export class Admin extends User {
  promote(other: User): Admin {
    function bump(n: number): number {
      return n + 1;
    }
    const level = bump(1);
    return new Admin(other.name + level, Role.Admin);
  }
}

export function validate(model: Base): boolean {
  return model !== null;
}

// WHY: names are displayed title-cased everywhere.
export const formatName = (name: string): string => name.toUpperCase();

export function helper(value: string): string {
  return value.trim();
}
