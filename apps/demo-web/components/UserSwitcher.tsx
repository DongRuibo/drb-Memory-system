"use client";

import type { DemoUserProfile } from "../app/types";

export function UserSwitcher({
  currentUser,
  users,
  disabled,
  onSwitch,
}: {
  currentUser: DemoUserProfile | null;
  users: DemoUserProfile[];
  disabled: boolean;
  onSwitch: (userId: string) => void;
}) {
  return (
    <section className="user-switcher" aria-label="切换演示用户">
      <label htmlFor="demo-user">当前演示身份</label>
      <div className="user-switcher-control">
        <span className="user-avatar">{currentUser?.avatar_initial ?? "?"}</span>
        <select
          disabled={disabled || !currentUser}
          id="demo-user"
          onChange={(event) => onSwitch(event.target.value)}
          value={currentUser?.user_id ?? ""}
        >
          {users.map((user) => (
            <option key={user.user_id} value={user.user_id}>
              {user.display_name} · {user.role_label}
            </option>
          ))}
        </select>
      </div>
      <p>
        {currentUser
          ? `${currentUser.role_label} · ${currentUser.primary_project_label}`
          : "正在建立演示会话…"}
      </p>
      <small>个人记忆彼此隔离；同项目成员共享项目事实。</small>
    </section>
  );
}
