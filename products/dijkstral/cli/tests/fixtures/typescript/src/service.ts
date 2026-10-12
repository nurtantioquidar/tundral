import "./polyfill";
import { readFileSync } from "fs";
import * as path from "node:path";
import Repo, { User, Admin as Administrator, helper as modelHelper, Storage } from "./models";
import type { Named } from "./models/user";
import { slugify } from "./util.js";

export function helper(value: string): string {
  return value.toLowerCase();
}

export class UserService {
  private repo: Repo<User>;

  constructor(repo: Repo<User>) {
    this.repo = repo;
  }

  register(name: string): User {
    const user = new User(name);
    user.save();
    this.repo.add(user);
    this.log(slugify(name));
    return user;
  }

  promote(admin: Administrator, user: Named & User): void {
    admin.promote(user);
  }

  export(): string[] {
    const names = this.repo.all().map((u) => u.name);
    names.sort(compareNames);
    return names.map(modelHelper).map(helper);
  }

  private log(message: string): void {
    console.log(Storage.key(message), path.basename(message));
  }
}

function compareNames(a: string, b: string): number {
  return a.localeCompare(b);
}

export async function run(file: string): Promise<UserService> {
  const service = new UserService(new Repo<User>());
  service.register(readFileSync(file, "utf8"));
  const lazy = await import("./util.js");
  lazy.slugify("x");
  unknownFunction();
  return service;
}
