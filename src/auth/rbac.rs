//! Role-Based Access Control (RBAC) helpers
//!
//! Provides optional RBAC functionality for applications that need it

use std::collections::HashSet;

/// User role
pub type Role = String;

/// Permission identifier
pub type Permission = String;

/// RBAC manager for role-based access control
pub struct RbacManager {
    user_roles: HashSet<Role>,
    role_permissions: std::collections::HashMap<Role, HashSet<Permission>>,
}

impl RbacManager {
    /// Create a new RBAC manager
    pub fn new() -> Self {
        Self {
            user_roles: HashSet::new(),
            role_permissions: std::collections::HashMap::new(),
        }
    }

    /// Assign a role to the user
    pub fn assign_role(&mut self, role: Role) {
        self.user_roles.insert(role);
    }

    /// Remove a role from the user
    pub fn remove_role(&mut self, role: &str) {
        self.user_roles.remove(role);
    }

    /// Grant a permission to a role
    pub fn grant_permission(&mut self, role: Role, permission: Permission) {
        self.role_permissions
            .entry(role)
            .or_insert_with(HashSet::new)
            .insert(permission);
    }

    /// Check if user has a specific role
    pub fn has_role(&self, role: &str) -> bool {
        self.user_roles.contains(role)
    }

    /// Check if user has a specific permission (via any of their roles)
    pub fn has_permission(&self, permission: &str) -> bool {
        self.user_roles.iter().any(|role| {
            self.role_permissions
                .get(role)
                .map(|perms| perms.contains(permission))
                .unwrap_or(false)
        })
    }

    /// Get all user roles
    pub fn roles(&self) -> &HashSet<Role> {
        &self.user_roles
    }
}

impl Default for RbacManager {
    fn default() -> Self {
        Self::new()
    }
}
