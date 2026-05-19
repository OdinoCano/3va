use rquickjs::{Ctx, Result};
use std::cell::RefCell;
use std::rc::Rc;
use vvva_permissions::PermissionState;

pub fn inject_fs(ctx: &Ctx, _permissions: Rc<RefCell<PermissionState>>) -> Result<()> {
    ctx.eval::<(), _>(
        r#"
        const fs = {
            readFile: function(path) {
                throw new Error('fs.readFile requires --allow-read');
            },
            writeFile: function(path, content) {
                throw new Error('fs.writeFile requires --allow-write');
            },
            exists: function(path) {
                return false;
            },
            readdir: function(path) {
                throw new Error('fs.readdir requires --allow-read');
            }
        };
        globalThis.fs = fs;
    "#,
    )?;
    Ok(())
}
