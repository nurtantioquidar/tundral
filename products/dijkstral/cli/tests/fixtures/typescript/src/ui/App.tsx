import React, { useState } from "react";
import { User } from "../models";
import { UserService, run } from "../service";

interface BadgeProps {
  user: User;
}

/** Shows a user's display name. */
export function Badge({ user }: BadgeProps): JSX.Element {
  return <span className="badge">{user.display}</span>;
}

export const UserList = ({ users }: { users: User[] }) => (
  <ul>
    {users.map((u) => (
      <li key={u.id}>
        <Badge user={u} />
      </li>
    ))}
  </ul>
);

export default class App extends React.Component<{ service: UserService }> {
  handleClick = (): void => {
    run("/tmp/users");
  };

  render() {
    const [users] = useState<User[]>([]);
    return (
      <div onClick={this.handleClick}>
        <UserList users={users} />
      </div>
    );
  }
}
