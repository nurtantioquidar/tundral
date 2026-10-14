Imports Shop.Models

Namespace Shop.Services
    Public Class UserService
        Private ReadOnly _users As New List(Of User)

        Public Function Register(name As String) As User
            Dim u As New User(name)
            _users.Add(u)
            Return u
        End Function
    End Class
End Namespace
