use anyhow::Result;

use super::process::run_cmd;

/// Add an IPv4 alias to lo0.
pub fn add_ipv4_alias(addr: &str) -> Result<()> {
    let cidr = format!("{addr}/32");
    run_cmd("ifconfig", &["lo0", "alias", &cidr])?;
    Ok(())
}

/// Remove an IPv4 alias from lo0.
pub fn remove_ipv4_alias(addr: &str) -> Result<()> {
    let _ = run_cmd("ifconfig", &["lo0", "-alias", addr]);
    Ok(())
}

/// Add an IPv6 alias to lo0.
pub fn add_ipv6_alias(addr: &str) -> Result<()> {
    run_cmd("ifconfig", &["lo0", "inet6", addr, "prefixlen", "128"])?;
    Ok(())
}

/// Remove an IPv6 alias from lo0.
pub fn remove_ipv6_alias(addr: &str) -> Result<()> {
    let _ = run_cmd("ifconfig", &["lo0", "inet6", "-alias", addr]);
    Ok(())
}

/// Manages a set of loopback aliases, removing them on drop.
pub struct LoopbackAliases {
    ipv4: Vec<String>,
    ipv6: Vec<String>,
}

impl LoopbackAliases {
    pub fn new() -> Self {
        Self {
            ipv4: Vec::new(),
            ipv6: Vec::new(),
        }
    }

    pub fn add_v4(&mut self, addr: &str) -> Result<()> {
        add_ipv4_alias(addr)?;
        self.ipv4.push(addr.to_string());
        Ok(())
    }

    pub fn add_v6(&mut self, addr: &str) -> Result<()> {
        add_ipv6_alias(addr)?;
        self.ipv6.push(addr.to_string());
        Ok(())
    }

    pub fn teardown(&mut self) {
        for addr in self.ipv4.drain(..) {
            let _ = remove_ipv4_alias(&addr);
        }
        for addr in self.ipv6.drain(..) {
            let _ = remove_ipv6_alias(&addr);
        }
    }
}

impl Drop for LoopbackAliases {
    fn drop(&mut self) {
        self.teardown();
    }
}
